use crate::{ExecError, RowSet};
use crumble_ir::ColumnDef as IrColumnDef;
use crumble_storage::Catalog;
use crumble_storage::{ColumnDef as StorageColumnDef, ColumnType as StorageColumnType};

pub(super) fn create(
    catalog: &mut Catalog,
    table: &str,
    columns: &[IrColumnDef],
) -> Result<RowSet, ExecError> {
    let storage_columns: Vec<StorageColumnDef> = columns
        .iter()
        .map(|c| StorageColumnDef {
            name: c.name.clone(),
            ty: match c.typ {
                crumble_ir::ColumnType::Int => StorageColumnType::Int,
                crumble_ir::ColumnType::Bool => StorageColumnType::Bool,
                crumble_ir::ColumnType::Text => StorageColumnType::String,
            },
        })
        .collect();

    catalog.create_table(table, storage_columns)?;
    Ok(RowSet::new(Vec::new(), Vec::new()))
}

pub(super) fn create_index(
    catalog: &mut Catalog,
    index_name: &str,
    table: &str,
    column: &str,
) -> Result<RowSet, ExecError> {
    catalog.create_index(index_name, table, column)?;
    Ok(RowSet::new(Vec::new(), Vec::new()))
}
