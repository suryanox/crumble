use crate::{ExecError, RowSet};
use crumble_storage::Catalog;
use crumble_tx::TransactionId;

pub(super) fn seqscan(
    catalog: &Catalog,
    table: &String,
    xid: TransactionId,
) -> Result<RowSet, ExecError> {
    let table_handle = catalog.table(table)?;
    let mut table = table_handle.lock().unwrap();
    let rows = table.rows(xid)?;
    let column_names: Vec<String> = table.columns().iter().map(|c| c.name.clone()).collect();
    Ok(RowSet::new(column_names, rows))
}
