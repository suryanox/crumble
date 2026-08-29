use crumble_ir::{Expr, Literal};
use crumble_storage::{Catalog, Row, Value, value_to_index_key};
use crumble_tx::TransactionId;

use crate::error::ExecError;
use crate::interpret::eval::{eval_expr, literal_to_value};
use crate::row_set::RowSet;

pub(super) fn update(
    catalog: &Catalog,
    table: &str,
    assignments: &[(String, Literal)],
    predicate: &Option<Expr>,
    xid: TransactionId,
) -> Result<RowSet, ExecError> {
    let target_handle = catalog.table(table)?;

    let (columns, located_rows) = {
        let mut target = target_handle.lock().unwrap();
        let located_rows = target.rows_with_location(xid)?;
        let columns: Vec<String> = target.columns().iter().map(|c| c.name.clone()).collect();
        (columns, located_rows)
    }; // physical lock released here

    let mut changed = Vec::new();

    for ((page_index, slot), row) in located_rows {
        let matches = match predicate {
            Some(expr) => matches!(eval_expr(expr, &columns, &row)?, Value::Bool(true)),
            None => true,
        };
        if !matches {
            continue;
        }

        let mut values = row.values().to_vec();
        for (col, literal) in assignments {
            let index = columns
                .iter()
                .position(|c| c == col)
                .ok_or_else(|| ExecError::ColumnNotFound(col.clone()))?;
            values[index] = literal_to_value(literal);
        }

        crumble_storage::delete_at(&target_handle, page_index, slot, xid)?;
        let (new_page_index, new_slot) = {
            let mut target = target_handle.lock().unwrap();
            target.insert(Row::new(values.clone()), xid)?
        };

        changed.push((
            page_index,
            slot,
            row,
            new_page_index,
            new_slot,
            Row::new(values),
        ));
    }

    let updated = changed.len() as i64;

    let indexed_columns: Vec<(usize, String)> = columns
        .iter()
        .enumerate()
        .filter_map(|(i, c)| catalog.index_for(table, c).map(|n| (i, n)))
        .collect();

    for (old_page, old_slot, old_row, new_page, new_slot, new_row) in &changed {
        for (col_pos, index_name) in &indexed_columns {
            if let Some(old_key) = value_to_index_key(&old_row.values()[*col_pos]) {
                let index_handle = catalog.index(index_name)?;
                index_handle
                    .lock()
                    .unwrap()
                    .delete(&old_key, *old_page, *old_slot)?;
            }
            if let Some(new_key) = value_to_index_key(&new_row.values()[*col_pos]) {
                let index_handle = catalog.index(index_name)?;
                index_handle
                    .lock()
                    .unwrap()
                    .insert(new_key, *new_page, *new_slot)?;
            }
        }
    }

    Ok(RowSet::new(
        vec!["updated".to_string()],
        vec![Row::new(vec![Value::Int(updated)])],
    ))
}
