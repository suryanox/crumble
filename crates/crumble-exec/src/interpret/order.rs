use crate::ExecError;
use crumble_storage::Value;

pub fn order_row_values(
    table_columns: &[String],
    insert_columns: &[String],
    values: Vec<Value>,
) -> Result<Vec<Value>, ExecError> {
    if insert_columns.is_empty() {
        return Ok(values);
    }

    if insert_columns.len() != table_columns.len() {
        return Err(ExecError::MissingColumn(
            "INSERT must specify all columns until NULL/defaults are supported".to_string(),
        ));
    }

    table_columns
        .iter()
        .map(|table_col| {
            let index = insert_columns
                .iter()
                .position(|c| c == table_col)
                .ok_or_else(|| ExecError::MissingColumn(table_col.clone()))?;
            Ok(values[index].clone())
        })
        .collect()
}
