use std::collections::HashMap;
use std::path::PathBuf;

use crumble_index::BTree;
use serde::{Deserialize, Serialize};

use crate::error::StorageError;
use crate::index_key::value_to_index_key;
use crate::table::Table;

#[derive(Debug, Default, Serialize, Deserialize)]
struct CatalogMeta {
    tables: HashMap<String, Vec<String>>,
    indexes: HashMap<String, IndexMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexMeta {
    table: String,
    column: String,
}

#[derive(Debug)]
pub struct Catalog {
    data_dir: PathBuf,
    tables: HashMap<String, Table>,
    indexes: HashMap<String, BTree>,
    index_meta: HashMap<String, IndexMeta>,
}

impl Catalog {
    pub fn open(data_dir: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let data_dir = data_dir.into();
        std::fs::create_dir_all(&data_dir)?;

        let meta = Self::load_meta(&data_dir)?;

        let mut tables = HashMap::new();
        for (name, columns) in &meta.tables {
            let table = Table::open(name.clone(), columns.clone(), &data_dir)?;
            tables.insert(name.clone(), table);
        }

        let mut indexes = HashMap::new();
        for name in meta.indexes.keys() {
            let tree = BTree::open(data_dir.join(format!("{name}.idx")))?;
            indexes.insert(name.clone(), tree);
        }

        Ok(Self {
            data_dir,
            tables,
            indexes,
            index_meta: meta.indexes,
        })
    }

    fn meta_path(data_dir: &std::path::Path) -> PathBuf {
        data_dir.join("catalog.json")
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
                .map(|(n, t)| (n.clone(), t.columns().to_vec()))
                .collect(),
            indexes: self.index_meta.clone(),
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

    pub fn create_index(
        &mut self,
        index_name: impl Into<String>,
        table: impl Into<String>,
        column: impl Into<String>,
    ) -> Result<(), StorageError> {
        let index_name = index_name.into();
        let table_name = table.into();
        let column = column.into();

        if self.index_meta.contains_key(&index_name) {
            return Err(StorageError::TableAlreadyExists(index_name));
        }

        let index_path = self.data_dir.join(format!("{index_name}.idx"));

        let target = self.get_mut(&table_name)?;
        let col_pos = target
            .columns()
            .iter()
            .position(|c| c == &column)
            .ok_or_else(|| StorageError::TableNotFound(format!("{table_name}.{column}")))?;

        let mut tree = BTree::open(index_path)?;

        for ((page_index, slot), row) in target.rows_with_location()? {
            if let Some(key) = value_to_index_key(&row.values()[col_pos]) {
                tree.insert(key, page_index, slot)?;
            }
        }

        self.indexes.insert(index_name.clone(), tree);
        self.index_meta.insert(
            index_name,
            IndexMeta {
                table: table_name,
                column,
            },
        );
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

    /// Finds an index covering (table, column), if one exists — this is
    /// what the optimizer rewrite step will call.
    pub fn index_for(&self, table: &str, column: &str) -> Option<&str> {
        self.index_meta
            .iter()
            .find(|(_, meta)| meta.table == table && meta.column == column)
            .map(|(name, _)| name.as_str())
    }

    pub fn index_mut(&mut self, index_name: &str) -> Result<&mut BTree, StorageError> {
        self.indexes
            .get_mut(index_name)
            .ok_or_else(|| StorageError::TableNotFound(index_name.to_string()))
    }
}
