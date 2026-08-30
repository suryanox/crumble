use crate::{ExecError, RowSet, execute};
use crumble_ir::{PhysicalPlan, Projection};
use crumble_storage::{Catalog, Row};
use crumble_tx::TransactionId;

pub(super) fn project(
    catalog: &Catalog,
    input: &Box<PhysicalPlan>,
    columns: &Projection,
    xid: TransactionId,
) -> Result<RowSet, ExecError> {
    let input = execute(input, catalog, xid)?;

    let resolved_columns: Vec<String> = match columns {
        Projection::All => input.columns().to_vec(),
        Projection::Columns(cols) => cols.clone(),
    };

    let mut indices = Vec::with_capacity(resolved_columns.len());
    for column in &resolved_columns {
        let index = input
            .column_index(column)
            .ok_or_else(|| ExecError::ColumnNotFound(column.clone()))?;
        indices.push(index);
    }

    let projected_rows = input
        .rows()
        .iter()
        .map(|row| {
            let values = indices.iter().map(|&i| row.values()[i].clone()).collect();
            Row::new(values)
        })
        .collect();

    Ok(RowSet::new(resolved_columns, projected_rows))
}
