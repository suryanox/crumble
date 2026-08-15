use crate::{ExecError, RowSet, execute};
use crumble_ir::PhysicalPlan;
use crumble_storage::{Catalog, Row};

pub(super) fn project(
    catalog: &mut Catalog,
    input: &Box<PhysicalPlan>,
    columns: &Vec<String>,
) -> Result<Result<RowSet, ExecError>, ExecError> {
    let input = execute(input, catalog)?;
    let mut indices = Vec::with_capacity(columns.len());

    for column in columns {
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

    Ok(Ok(RowSet::new(columns.clone(), projected_rows)))
}
