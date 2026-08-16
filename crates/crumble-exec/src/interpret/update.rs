use crumble_ir::{Expr, Literal};
use crumble_storage::{Catalog, Row, Value, value_to_index_key};

use crate::error::ExecError;
use crate::interpret::eval::{eval_expr, literal_to_value};
use crate::row_set::RowSet;

/**
even for a column that wasn't assigned in SET, its old and new value are identical, so delete(old_key,...) then insert(new_key,...) with old_key == new_key is a harmless no-op pair — correct, just not the cheapest possible path.
Fine for now; optimizing "only touch indexes on columns that actually changed" is a real but separate refinement, not required for correctness.

*/
pub(super) fn update(
    catalog: &mut Catalog,
    table: &str,
    assignments: &[(String, Literal)],
    predicate: &Option<Expr>,
) -> Result<RowSet, ExecError> {
    let target = catalog.get_mut(table)?;
    let columns = target.columns().to_vec();
    let located_rows = target.rows_with_location()?;

    // (old_page_index, old_slot, old_row, new_page_index, new_slot, new_row)
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

        target.delete_at(page_index, slot)?;
        let (new_page_index, new_slot) = target.insert(Row::new(values.clone()))?;

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
    // target's borrow of catalog ends here.

    let indexed_columns: Vec<(usize, String)> = columns
        .iter()
        .enumerate()
        .filter_map(|(i, c)| catalog.index_for(table, c).map(|n| (i, n.to_string())))
        .collect();

    for (old_page, old_slot, old_row, new_page, new_slot, new_row) in &changed {
        for (col_pos, index_name) in &indexed_columns {
            if let Some(old_key) = value_to_index_key(&old_row.values()[*col_pos]) {
                catalog
                    .index_mut(index_name)?
                    .delete(&old_key, *old_page, *old_slot)?;
            }
            if let Some(new_key) = value_to_index_key(&new_row.values()[*col_pos]) {
                catalog
                    .index_mut(index_name)?
                    .insert(new_key, *new_page, *new_slot)?;
            }
        }
    }

    Ok(RowSet::new(
        vec!["updated".to_string()],
        vec![Row::new(vec![Value::Int(updated)])],
    ))
}
