use crate::column::ColumnDef;
use crate::error::StorageError;
use crate::index_key::value_to_index_key;
use crate::table::Table;
use crumble_index::BTree;
use crumble_tx::TransactionManager;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

#[derive(Debug, Default, Serialize, Deserialize)]
struct CatalogMeta {
    tables: HashMap<String, Vec<ColumnDef>>,
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
    tables: RwLock<HashMap<String, Arc<Mutex<Table>>>>,
    indexes: RwLock<HashMap<String, Arc<Mutex<BTree>>>>,
    index_meta: RwLock<HashMap<String, IndexMeta>>,
    pub tx_manager: Arc<TransactionManager>,
}

impl Catalog {
    pub fn open(
        data_dir: impl Into<PathBuf>,
        tx_manager: Arc<TransactionManager>,
    ) -> Result<Self, StorageError> {
        let data_dir = data_dir.into();
        std::fs::create_dir_all(&data_dir)?;

        let meta = Self::load_meta(&data_dir)?;

        let mut tables = HashMap::new();
        for (name, columns) in &meta.tables {
            let table = Table::open(name.clone(), columns.clone(), &data_dir, tx_manager.clone())?;
            tables.insert(name.clone(), Arc::new(Mutex::new(table)));
        }

        let mut indexes = HashMap::new();
        for name in meta.indexes.keys() {
            let tree = BTree::open(name, &data_dir)?;
            indexes.insert(name.clone(), Arc::new(Mutex::new(tree)));
        }

        Ok(Self {
            data_dir,
            tables: RwLock::new(tables),
            indexes: RwLock::new(indexes),
            index_meta: RwLock::new(meta.indexes),
            tx_manager,
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
        let tables = self.tables.read().unwrap();
        let index_meta = self.index_meta.read().unwrap();

        let mut table_columns = HashMap::new();
        for (name, table) in tables.iter() {
            table_columns.insert(name.clone(), table.lock().unwrap().columns().to_vec());
        }

        let meta = CatalogMeta {
            tables: table_columns,
            indexes: index_meta.clone(),
        };
        drop(tables);
        drop(index_meta);

        let contents = serde_json::to_string_pretty(&meta)
            .map_err(|e| StorageError::Encoding(e.to_string()))?;
        std::fs::write(Self::meta_path(&self.data_dir), contents)?;
        Ok(())
    }

    pub fn create_table(
        &self,
        name: impl Into<String>,
        columns: Vec<ColumnDef>,
    ) -> Result<(), StorageError> {
        let name = name.into();
        let mut tables = self.tables.write().unwrap();

        if tables.contains_key(&name) {
            return Err(StorageError::TableAlreadyExists(name));
        }

        let table = Table::open(
            name.clone(),
            columns,
            &self.data_dir,
            self.tx_manager.clone(),
        )?;
        tables.insert(name, Arc::new(Mutex::new(table)));
        drop(tables);
        self.save_meta()
    }

    pub fn create_index(
        &self,
        index_name: impl Into<String>,
        table: impl Into<String>,
        column: impl Into<String>,
    ) -> Result<(), StorageError> {
        let index_name = index_name.into();
        let table_name = table.into();
        let column = column.into();

        {
            let index_meta = self.index_meta.read().unwrap();
            if index_meta.contains_key(&index_name) {
                return Err(StorageError::TableAlreadyExists(index_name));
            }
        }

        let mut tree = BTree::open(&index_name, &self.data_dir)?;

        let table_handle = self.table(&table_name)?;
        let mut target = table_handle.lock().unwrap();
        let col_pos = target
            .columns()
            .iter()
            .position(|c| c.name == column)
            .ok_or_else(|| StorageError::TableNotFound(format!("{table_name}.{column}")))?;

        for ((page_index, slot), row) in target.rows_with_location(u64::MAX)? {
            if let Some(key) = value_to_index_key(&row.values()[col_pos]) {
                tree.insert(key, page_index, slot)?;
            }
        }
        drop(target);

        self.indexes
            .write()
            .unwrap()
            .insert(index_name.clone(), Arc::new(Mutex::new(tree)));
        self.index_meta.write().unwrap().insert(
            index_name,
            IndexMeta {
                table: table_name,
                column,
            },
        );
        self.save_meta()
    }

    /// Returns a shared handle to the table — caller locks it when ready to use it.
    pub fn table(&self, name: &str) -> Result<Arc<Mutex<Table>>, StorageError> {
        self.tables
            .read()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| StorageError::TableNotFound(name.to_string()))
    }

    /// Finds an index covering (table, column), if one exists — this is
    /// what the optimizer rewrite step will call.
    pub fn index_for(&self, table: &str, column: &str) -> Option<String> {
        self.index_meta
            .read()
            .unwrap()
            .iter()
            .find(|(_, meta)| meta.table == table && meta.column == column)
            .map(|(name, _)| name.clone())
    }

    pub fn index(&self, name: &str) -> Result<Arc<Mutex<BTree>>, StorageError> {
        self.indexes
            .read()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| StorageError::TableNotFound(name.to_string()))
    }
}
