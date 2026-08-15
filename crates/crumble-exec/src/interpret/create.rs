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
