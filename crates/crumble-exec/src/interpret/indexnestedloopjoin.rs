use std::collections::HashSet;

use crumble_ir::{JoinKind, PhysicalPlan};
use crumble_storage::{Catalog, Row, Value, value_to_index_key};
use crumble_tx::TransactionId;

use crate::error::ExecError;
use crate::execute;
use crate::row_set::RowSet;

pub(super) fn indexnestedloopjoin(
    catalog: &Catalog,
    left: &Box<PhysicalPlan>,
    left_table: &str,
    right_table_real: &str,
    right_table_qualifier: &str,
    right_index_name: &str,
    left_join_column: &str,
    kind: &JoinKind,
    xid: TransactionId,
) -> Result<RowSet, ExecError> {
    let left_result = execute(left, catalog, xid)?;

    let probe_index = left_result
        .column_index(left_join_column)
        .ok_or_else(|| ExecError::ColumnNotFound(left_join_column.to_string()))?;

    let right_table_handle = catalog.table(right_table_real)?;
    let right_columns: Vec<String> = right_table_handle
        .lock()
        .unwrap()
        .columns()
        .iter()
        .map(|c| c.name.clone())
        .collect();
    let right_width = right_columns.len();
    let left_width = left_result.columns().len();

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

    let needs_right_unmatched = matches!(kind, JoinKind::Right | JoinKind::FullOuter);
    let mut matched_right_locations: HashSet<(u32, u16)> = HashSet::new();

    let mut output_rows = Vec::new();

    for left_row in left_result.rows() {
        let probe_value = &left_row.values()[probe_index];

        let matches = match value_to_index_key(probe_value) {
            Some(key) => {
                let index_handle = catalog.index(right_index_name)?;
                index_handle.lock().unwrap().search(&key)?
            }
            None => Vec::new(),
        };

        let mut matched_any = false;
        for (page_index, slot) in matches {
            let right_table_handle = catalog.table(right_table_real)?;
            let mut right_table = right_table_handle.lock().unwrap();
            if let Some(right_row) = right_table.row_at(page_index, slot, xid)? {
                let mut combined_values = left_row.values().to_vec();
                combined_values.extend(right_row.values().iter().cloned());
                output_rows.push(Row::new(combined_values));
                matched_any = true;

                if needs_right_unmatched {
                    matched_right_locations.insert((page_index, slot));
                }
            }
        }

        if !matched_any && matches!(kind, JoinKind::Left | JoinKind::FullOuter) {
            let mut padded_values = left_row.values().to_vec();
            padded_values.extend(std::iter::repeat(Value::Null).take(right_width));
            output_rows.push(Row::new(padded_values));
        }
    }

    if needs_right_unmatched {
        let right_table_handle = catalog.table(right_table_real)?;
        let mut right_table = right_table_handle.lock().unwrap();
        for ((page_index, slot), right_row) in right_table.rows_with_location(xid)? {
            if !matched_right_locations.contains(&(page_index, slot)) {
                let mut padded_values: Vec<Value> =
                    std::iter::repeat(Value::Null).take(left_width).collect();
                padded_values.extend(right_row.values().iter().cloned());
                output_rows.push(Row::new(padded_values));
            }
        }
    }

    Ok(RowSet::new(qualified_columns, output_rows))
}
