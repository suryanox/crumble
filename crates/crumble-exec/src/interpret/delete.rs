use crumble_ir::Expr;
use crumble_storage::{Catalog, Row, Value, value_to_index_key};

use crate::error::ExecError;
use crate::interpret::eval::eval_expr;
use crate::row_set::RowSet;

pub(super) fn delete(
    catalog: &mut Catalog,
    table: &str,
    predicate: &Option<Expr>,
) -> Result<RowSet, ExecError> {
    let target = catalog.get_mut(table)?;
    let located_rows = target.rows_with_location()?;
    let columns: Vec<String> = target.columns().iter().map(|c| c.name.clone()).collect();
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
        target.delete_at(*page_index, *slot)?;
    }
    let deleted = to_delete.len() as i64;
    // target's borrow of catalog ends here — not used again below.

    let indexed_columns: Vec<(usize, String)> = columns
        .iter()
        .enumerate()
        .filter_map(|(i, c)| catalog.index_for(table, c).map(|n| (i, n.to_string())))
        .collect();

    for (page_index, slot, row) in &to_delete {
        for (col_pos, index_name) in &indexed_columns {
            if let Some(key) = value_to_index_key(&row.values()[*col_pos]) {
                catalog
                    .index_mut(index_name)?
                    .delete(&key, *page_index, *slot)?;
            }
        }
    }

    Ok(RowSet::new(
        vec!["deleted".to_string()],
        vec![Row::new(vec![Value::Int(deleted)])],
    ))
}
