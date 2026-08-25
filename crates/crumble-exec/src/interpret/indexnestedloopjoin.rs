use crate::{ExecError, RowSet, execute};
use crumble_ir::{JoinKind, PhysicalPlan};
use crumble_storage::{Catalog, Row, Value, value_to_index_key};

pub(super) fn indexnestedloopjoin(
    catalog: &mut Catalog,
    left: &Box<PhysicalPlan>,
    left_table: &str,
    right_table_real: &str,
    right_table_qualifier: &str,
    right_index_name: &str,
    left_join_column: &str,
    kind: &JoinKind,
) -> Result<RowSet, ExecError> {
    let left_result = execute(left, catalog)?;

    let probe_index = left_result
        .column_index(left_join_column)
        .ok_or_else(|| ExecError::ColumnNotFound(left_join_column.to_string()))?;

    let right_columns: Vec<String> = catalog
        .get(right_table_real)?
        .columns()
        .iter()
        .map(|c| c.name.clone())
        .collect();

    let right_width = right_columns.len();

    let qualified_columns: Vec<String> = left_result
        .columns()
        .iter()
        .map(|c| format!("{left_table}.{c}"))
        .chain(
            right_columns
                .iter()
                .map(|c| format!("{right_table_qualifier}.{c}")),
        )
        .collect();

    let mut output_rows = Vec::new();

    for left_row in left_result.rows() {
        let probe_value = &left_row.values()[probe_index];

        let matches = match value_to_index_key(probe_value) {
            Some(key) => catalog.index_mut(right_index_name)?.search(&key)?,
            None => Vec::new(),
        };

        let mut matched_any = false;
        for (page_index, slot) in matches {
            let right_table = catalog.get_mut(right_table_real)?;
            if let Some(right_row) = right_table.row_at(page_index, slot)? {
                let mut combined_values = left_row.values().to_vec();
                combined_values.extend(right_row.values().iter().cloned());
                output_rows.push(Row::new(combined_values));
                matched_any = true;
            }
        }

        if !matched_any && *kind == JoinKind::Left {
            let mut padded_values = left_row.values().to_vec();
            padded_values.extend(std::iter::repeat(Value::Null).take(right_width));
            output_rows.push(Row::new(padded_values));
        }
    }

    Ok(RowSet::new(qualified_columns, output_rows))
}
