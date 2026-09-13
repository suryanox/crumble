use crate::{ExecError, RowSet, execute};
use crumble_ir::{PhysicalPlan, SortDirection};
use crumble_storage::{Catalog, Value};
use crumble_tx::TransactionId;
use std::cmp::Ordering;

pub(super) fn sort(
    catalog: &Catalog,
    input: &Box<PhysicalPlan>,
    order_by: &[(String, SortDirection)],
    xid: TransactionId,
) -> Result<RowSet, ExecError> {
    let input_result = execute(input, catalog, xid)?;

    let sort_indices: Vec<(usize, SortDirection)> = order_by
        .iter()
        .map(|(col, dir)| {
            input_result
                .column_index(col)
                .map(|i| (i, *dir))
                .ok_or_else(|| ExecError::ColumnNotFound(col.clone()))
        })
        .collect::<Result<_, _>>()?;

    let mut rows = input_result.rows().to_vec();
    rows.sort_by(|a, b| {
        for &(idx, dir) in &sort_indices {
            let ord = compare_with_direction(&a.values()[idx], &b.values()[idx], dir);
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    });

    Ok(RowSet::new(input_result.columns().to_vec(), rows))
}

/// NULLs always sort last, regardless of ASC/DESC — decided BEFORE any
/// direction reversal, since reversing the null-placement result along with
/// the real comparison would wrongly flip NULLs to the front under DESC.
fn compare_with_direction(a: &Value, b: &Value, dir: SortDirection) -> Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Greater,
        (_, Value::Null) => Ordering::Less,
        (a, b) => {
            let ord = compare_non_null(a, b);
            if dir == SortDirection::Desc {
                ord.reverse()
            } else {
                ord
            }
        }
    }
}

fn compare_non_null(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => a.cmp(b),
        (Value::Float(a), Value::Float(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
        (Value::String(a), Value::String(b)) => a.cmp(b),
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        _ => Ordering::Equal,
    }
}
