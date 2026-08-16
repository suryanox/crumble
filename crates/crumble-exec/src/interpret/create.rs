use crate::{ExecError, RowSet};
use crumble_storage::Catalog;

pub(super) fn create(
    catalog: &mut Catalog,
    table: &String,
    columns: &Vec<String>,
) -> Result<RowSet, ExecError> {
    catalog.create_table(table, columns.clone())?;
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
