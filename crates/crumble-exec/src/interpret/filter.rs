use crate::interpret::eval::eval_expr;
use crate::{ExecError, RowSet, execute};
use crumble_ir::{Expr, PhysicalPlan};
use crumble_storage::{Catalog, Value};
use crumble_tx::TransactionId;

pub(super) fn filter(
    catalog: &Catalog,
    input: &Box<PhysicalPlan>,
    predicate: &Expr,
    xid: TransactionId,
) -> Result<RowSet, ExecError> {
    let input = execute(input, catalog, xid)?;
    let mut kept = Vec::new();

    for row in input.rows() {
        let value = eval_expr(predicate, input.columns(), row)?;
        match value {
            Value::Bool(true) => kept.push(row.clone()),
            Value::Bool(false) | Value::Null => {}
            _ => return Err(ExecError::TypeMismatch),
        }
    }

    Ok(RowSet::new(input.columns().to_vec(), kept))
}
