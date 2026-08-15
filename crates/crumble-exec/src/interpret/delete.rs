use crate::interpret::eval::eval_expr;
use crate::{ExecError, RowSet};
use crumble_ir::Expr;
use crumble_storage::{Catalog, Row, Value};

pub(super) fn delete(
    catalog: &mut Catalog,
    table: &String,
    predicate: &Option<Expr>,
) -> Result<RowSet, ExecError> {
    let target = catalog.get_mut(table)?;
    let located_rows = target.rows_with_location()?;

    let mut deleted = 0;

    for ((page_index, slot), row) in located_rows {
        let matches = match predicate {
            Some(expr) => {
                matches!(eval_expr(expr, target.columns(), &row)?, Value::Bool(true))
            }
            None => true,
        };

        if matches {
            target.delete_at(page_index, slot)?;
            deleted += 1;
        }
    }

    Ok(RowSet::new(
        vec!["deleted".to_string()],
        vec![Row::new(vec![Value::Int(deleted)])],
    ))
}
