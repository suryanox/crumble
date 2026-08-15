use crate::interpret::eval::{eval_expr, literal_to_value};
use crate::{ExecError, RowSet};
use crumble_ir::{Expr, Literal};
use crumble_storage::{Catalog, Row, Value};

pub(super) fn update(
    catalog: &mut Catalog,
    table: &String,
    assignments: &Vec<(String, Literal)>,
    predicate: &Option<Expr>,
) -> Result<RowSet, ExecError> {
    let target = catalog.get_mut(table)?;
    let columns = target.columns().to_vec();
    let located_rows = target.rows_with_location()?;

    let mut updated = 0;

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
        target.insert(Row::new(values))?;
        updated += 1;
    }

    Ok(RowSet::new(
        vec!["updated".to_string()],
        vec![Row::new(vec![Value::Int(updated)])],
    ))
}
