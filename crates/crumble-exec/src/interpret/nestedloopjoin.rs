use crate::interpret::eval::eval_expr;
use crate::{ExecError, RowSet, execute};
use crumble_ir::{Expr, JoinKind, PhysicalPlan};
use crumble_storage::{Catalog, Row, Value};

pub(super) fn nestedloopjoin(
    catalog: &mut Catalog,
    left: &Box<PhysicalPlan>,
    right: &Box<PhysicalPlan>,
    left_table: &str,
    right_table: &str,
    on: &Expr,
    kind: &JoinKind,
) -> Result<RowSet, ExecError> {
    let left_result = execute(left, catalog)?;
    let right_result = execute(right, catalog)?;

    let qualified_columns: Vec<String> = left_result
        .columns()
        .iter()
        .map(|c| format!("{left_table}.{c}"))
        .chain(
            right_result
                .columns()
                .iter()
                .map(|c| format!("{right_table}.{c}")),
        )
        .collect();

    let mut matched_rows = Vec::new();

    let right_width = right_result.columns().len();

    for left_row in left_result.rows() {
        let mut matched_any = false;
        for right_row in right_result.rows() {
            let mut combined_values = left_row.values().to_vec();
            combined_values.extend(right_row.values().iter().cloned());
            let combined_row = Row::new(combined_values);

            let is_match = matches!(
                eval_expr(on, &qualified_columns, &combined_row)?,
                Value::Bool(true)
            );

            if is_match {
                matched_any = true;
                matched_rows.push(combined_row);
            }
        }

        if !matched_any && *kind == JoinKind::Left {
            let mut padded_values = left_row.values().to_vec();
            padded_values.extend(std::iter::repeat(Value::Null).take(right_width));
            matched_rows.push(Row::new(padded_values));
        }
    }

    Ok(RowSet::new(qualified_columns, matched_rows))
}
