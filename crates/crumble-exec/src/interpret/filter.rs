use crumble_ir::{Expr, PhysicalPlan};
use crumble_jit::compile_predicate;
use crumble_storage::{Catalog, Row, Value};
use crumble_tx::TransactionId;
use inkwell::context::Context;
use std::collections::HashMap;

use crate::error::ExecError;
use crate::execute;
use crate::interpret::eval::eval_expr;
use crate::row_set::RowSet;

pub(super) fn filter(
    catalog: &Catalog,
    input: &Box<PhysicalPlan>,
    predicate: &Expr,
    xid: TransactionId,
) -> Result<RowSet, ExecError> {
    let input = execute(input, catalog, xid)?;

    let mut column_index: HashMap<String, usize> = HashMap::new();
    if let Some(first_row) = input.rows().first() {
        for (i, col) in input.columns().iter().enumerate() {
            if matches!(first_row.values()[i], Value::Int(_)) {
                column_index.insert(col.clone(), i);
            }
        }
    }

    let context = Context::create();
    let compiled = compile_predicate(&context, predicate, &column_index);

    let mut kept = Vec::new();

    for row in input.rows() {
        let matched = match &compiled {
            Some(compiled) if row_is_all_int(row, &column_index) => {
                let flat: Vec<i64> = row
                    .values()
                    .iter()
                    .map(|v| if let Value::Int(n) = v { *n } else { 0 })
                    .collect();
                unsafe { compiled.call(flat.as_ptr()) }
            }
            _ => {
                let value = eval_expr(predicate, input.columns(), row)?;
                match value {
                    Value::Bool(b) => b,
                    Value::Null => false,
                    _ => return Err(ExecError::TypeMismatch),
                }
            }
        };

        if matched {
            kept.push(row.clone());
        }
    }

    Ok(RowSet::new(input.columns().to_vec(), kept))
}

/// True only if every column the compiled predicate might read is
/// genuinely Value::Int in THIS row — guards against silently treating a
/// NULL as a fake 0 in the fast path.
fn row_is_all_int(row: &Row, column_index: &HashMap<String, usize>) -> bool {
    column_index
        .values()
        .all(|&i| matches!(row.values()[i], Value::Int(_)))
}
