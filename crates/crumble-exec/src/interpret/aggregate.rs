use crumble_ir::{AggFunc, AggregateExpr, PhysicalPlan};
use crumble_storage::{Catalog, Row, Value};
use crumble_tx::TransactionId;

use crate::error::ExecError;
use crate::execute;
use crate::row_set::RowSet;

pub(super) fn aggregate(
    catalog: &Catalog,
    input: &Box<PhysicalPlan>,
    group_by: &[String],
    aggregates: &[AggregateExpr],
    xid: TransactionId,
) -> Result<RowSet, ExecError> {
    let input_result = execute(input, catalog, xid)?;

    let group_indices: Vec<usize> = group_by
        .iter()
        .map(|c| {
            input_result
                .column_index(c)
                .ok_or_else(|| ExecError::ColumnNotFound(c.clone()))
        })
        .collect::<Result<_, _>>()?;

    let agg_indices: Vec<Option<usize>> = aggregates
        .iter()
        .map(|a| match &a.column {
            Some(col) => input_result
                .column_index(col)
                .map(Some)
                .ok_or_else(|| ExecError::ColumnNotFound(col.clone())),
            None => Ok(None),
        })
        .collect::<Result<_, _>>()?;

    let mut groups: Vec<(Vec<Value>, Vec<Row>)> = Vec::new();
    for row in input_result.rows() {
        let key: Vec<Value> = group_indices
            .iter()
            .map(|&i| row.values()[i].clone())
            .collect();
        match groups.iter_mut().find(|(k, _)| k == &key) {
            Some((_, rows)) => rows.push(row.clone()),
            None => groups.push((key, vec![row.clone()])),
        }
    }

    // No GROUP BY at all still means one implicit group over everything —
    // "SELECT COUNT(*) FROM empty_table" must return one row (count=0),
    // not zero rows, even with no input rows at all.
    if group_by.is_empty() && groups.is_empty() {
        groups.push((Vec::new(), Vec::new()));
    }

    let output_columns: Vec<String> = group_by
        .iter()
        .cloned()
        .chain(aggregates.iter().map(|a| a.alias.clone()))
        .collect();

    let mut output_rows = Vec::new();
    for (key, rows) in &groups {
        let mut values = key.clone();
        for (agg, &agg_idx) in aggregates.iter().zip(agg_indices.iter()) {
            values.push(compute_aggregate(agg, agg_idx, rows)?);
        }
        output_rows.push(Row::new(values));
    }

    Ok(RowSet::new(output_columns, output_rows))
}

fn compute_aggregate(
    agg: &AggregateExpr,
    col_index: Option<usize>,
    rows: &[Row],
) -> Result<Value, ExecError> {
    match agg.func {
        AggFunc::Count => {
            let count = match col_index {
                None => rows.len(),
                Some(i) => rows.iter().filter(|r| r.values()[i] != Value::Null).count(),
            };
            Ok(Value::Int(count as i64))
        }
        AggFunc::Sum => {
            let non_null = non_null_values(col_index, rows);
            if non_null.is_empty() {
                return Ok(Value::Null);
            }
            sum_values(&non_null)
        }
        AggFunc::Avg => {
            let non_null = non_null_values(col_index, rows);
            if non_null.is_empty() {
                return Ok(Value::Null);
            }
            let sum = sum_values(&non_null)?;
            let count = non_null.len() as f64;
            match sum {
                Value::Int(n) => Ok(Value::Float(n as f64 / count)),
                Value::Float(f) => Ok(Value::Float(f / count)),
                _ => unreachable!("sum_values only ever returns Int or Float"),
            }
        }
        AggFunc::Min | AggFunc::Max => {
            let non_null = non_null_values(col_index, rows);
            if non_null.is_empty() {
                return Ok(Value::Null);
            }
            extremum(&non_null, agg.func == AggFunc::Max)
        }
    }
}

fn non_null_values(col_index: Option<usize>, rows: &[Row]) -> Vec<Value> {
    let i = col_index.expect("SUM/AVG/MIN/MAX always have a column, enforced at lowering");
    rows.iter()
        .map(|r| r.values()[i].clone())
        .filter(|v| *v != Value::Null)
        .collect()
}

fn sum_values(values: &[Value]) -> Result<Value, ExecError> {
    let all_int = values.iter().all(|v| matches!(v, Value::Int(_)));
    if all_int {
        let total: i64 = values
            .iter()
            .map(|v| match v {
                Value::Int(n) => *n,
                _ => 0,
            })
            .sum();
        return Ok(Value::Int(total));
    }

    let mut total = 0.0f64;
    for v in values {
        match v {
            Value::Int(n) => total += *n as f64,
            Value::Float(f) => total += *f,
            _ => return Err(ExecError::TypeMismatch),
        }
    }
    Ok(Value::Float(total))
}

fn extremum(values: &[Value], want_max: bool) -> Result<Value, ExecError> {
    let mut best = values[0].clone();
    for v in &values[1..] {
        let is_better = match (v, &best) {
            (Value::Int(a), Value::Int(b)) => {
                if want_max {
                    a > b
                } else {
                    a < b
                }
            }
            (Value::Float(a), Value::Float(b)) => {
                if want_max {
                    a > b
                } else {
                    a < b
                }
            }
            (Value::String(a), Value::String(b)) => {
                if want_max {
                    a > b
                } else {
                    a < b
                }
            }
            _ => return Err(ExecError::TypeMismatch),
        };
        if is_better {
            best = v.clone();
        }
    }
    Ok(best)
}
