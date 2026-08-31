use crumble_ir::Expr;
use crumble_storage::{Catalog, Row, Value, delete_at};
use crumble_tx::TransactionId;

use crate::error::ExecError;
use crate::interpret::eval::eval_expr;
use crate::row_set::RowSet;

pub(super) fn delete(
    catalog: &Catalog,
    table: &str,
    predicate: &Option<Expr>,
    xid: TransactionId,
) -> Result<RowSet, ExecError> {
    let target_handle = catalog.table(table)?;

    let (columns, located_rows) = {
        let mut target = target_handle.lock().unwrap();
        let located_rows = target.rows_with_location(xid)?;
        let columns: Vec<String> = target.columns().iter().map(|c| c.name.clone()).collect();
        (columns, located_rows)
    };

    let mut to_delete: Vec<(u32, u16, Row)> = Vec::new();
    for ((page_index, slot), row) in located_rows {
        let matches = match predicate {
            Some(expr) => matches!(eval_expr(expr, &columns, &row)?, Value::Bool(true)),
            None => true,
        };
        if matches {
            to_delete.push((page_index, slot, row));
        }
    }

    for (page_index, slot, _) in &to_delete {
        delete_at(&target_handle, *page_index, *slot, xid)?;
    }
    let deleted = to_delete.len() as i64;

    // Note: no index cleanup here, deliberately. A deleted row's index
    // entries become dead-but-harmless — is_visible() correctly filters
    // them out at read time via row_at, same as Postgres, which also never
    // removes index entries at DELETE time (only VACUUM does, once no
    // transaction could possibly need the old version). Eagerly removing
    // here was the actual bug: an aborted DELETE would leave the index
    // permanently out of sync with the table, since nothing re-added the
    // entry on rollback.

    Ok(RowSet::new(
        vec!["deleted".to_string()],
        vec![Row::new(vec![Value::Int(deleted)])],
    ))
}
