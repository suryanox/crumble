use crate::{ExecError, RowSet};
use crumble_storage::Catalog;

pub(super) fn seqscan(catalog: &mut Catalog, table: &String) -> Result<RowSet, ExecError> {
    let table = catalog.get_mut(table)?;
    let rows = table.rows()?;
    Ok(RowSet::new(table.columns().to_vec(), rows))
}
