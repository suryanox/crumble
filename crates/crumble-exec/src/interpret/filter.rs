use std::collections::HashMap;

use crumble_ir::{Expr, PhysicalPlan};
use crumble_jit::{ColumnKind, compile_predicate};
use crumble_storage::{Catalog, Row, Value};
use crumble_tx::TransactionId;
use inkwell::context::Context;

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

    let mut column_index: HashMap<String, (usize, ColumnKind)> = HashMap::new();
    if let Some(first_row) = input.rows().first() {
        for (i, col) in input.columns().iter().enumerate() {
            match first_row.values()[i] {
                Value::Int(_) => {
                    column_index.insert(col.clone(), (i, ColumnKind::Int));
                }
                Value::Float(_) => {
                    column_index.insert(col.clone(), (i, ColumnKind::Float));
                }
                _ => {}
            }
        }
    }

    let context = Context::create();
    let compiled = compile_predicate(&context, predicate, &column_index);

    let mut kept = Vec::new();

    for row in input.rows() {
        let matched = match &compiled {
            Some(compiled) => {
                let (values, nulls) = build_flat_arrays(row, &column_index);
                unsafe { compiled.call(values.as_ptr(), nulls.as_ptr()) }
            }
            None => {
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

/// Builds the two flat arrays a compiled predicate reads from: raw i64 bits
/// per tracked column (Float values bitcast into the same 8 bytes an Int
/// would occupy), and a parallel 0/1 null flag per column.
fn build_flat_arrays(
    row: &Row,
    column_index: &HashMap<String, (usize, ColumnKind)>,
) -> (Vec<i64>, Vec<i64>) {
    let width = column_index.values().map(|(i, _)| i + 1).max().unwrap_or(0);
    let mut values = vec![0i64; width];
    let mut nulls = vec![0i64; width];

    for &(idx, kind) in column_index.values() {
        match (&row.values()[idx], kind) {
            (Value::Int(n), ColumnKind::Int) => values[idx] = *n,
            (Value::Float(f), ColumnKind::Float) => values[idx] = f.to_bits() as i64,
            (Value::Null, _) => nulls[idx] = 1,
            _ => {} // schema mismatch shouldn't happen given typed columns
        }
    }

    (values, nulls)
}
