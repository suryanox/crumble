use crate::{ExecError, RowSet};
use crumble_storage::Catalog;

pub(super) fn seqscan(catalog: &mut Catalog, table: &String) -> Result<RowSet, ExecError> {
    let table = catalog.get_mut(table)?;
    let rows = table.rows()?;
    let column_names: Vec<String> = table.columns().iter().map(|c| c.name.clone()).collect();
    Ok(RowSet::new(column_names, rows))
}
