use crumble_ir::Expr;
use crumble_storage::{Catalog, Row, Value, delete_at, value_to_index_key};
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
    }; // physical lock released here — matching below touches no shared state

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

    let indexed_columns: Vec<(usize, String)> = columns
        .iter()
        .enumerate()
        .filter_map(|(i, c)| catalog.index_for(table, c).map(|n| (i, n)))
        .collect();

    for (page_index, slot, row) in &to_delete {
        for (col_pos, index_name) in &indexed_columns {
            if let Some(key) = value_to_index_key(&row.values()[*col_pos]) {
                let index_handle = catalog.index(index_name)?;
                index_handle
                    .lock()
                    .unwrap()
                    .delete(&key, *page_index, *slot)?;
            }
        }
    }

    Ok(RowSet::new(
        vec!["deleted".to_string()],
        vec![Row::new(vec![Value::Int(deleted)])],
    ))
}
