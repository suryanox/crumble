use crate::{ExecError, RowSet};
use crumble_storage::Catalog;

pub(super) fn drop_table(
    catalog: &Catalog,
    table: &str,
    if_exists: bool,
) -> Result<RowSet, ExecError> {
    catalog.drop_table(table, if_exists)?;
    Ok(RowSet::new(Vec::new(), Vec::new()))
}

pub(super) fn drop_index(
    catalog: &Catalog,
    index_name: &str,
    if_exists: bool,
) -> Result<RowSet, ExecError> {
    catalog.drop_index(index_name, if_exists)?;
    Ok(RowSet::new(Vec::new(), Vec::new()))
}
