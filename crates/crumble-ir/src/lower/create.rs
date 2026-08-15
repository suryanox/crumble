use sqlparser::ast::CreateTable;
use crate::{LogicalPlan, LowerError};

pub(super) fn lower_create(create_table: &CreateTable) -> Result<LogicalPlan, LowerError> {
    let table = create_table.name.to_string();

    let columns = create_table
        .columns
        .iter()
        .map(|col| col.name.value.clone())
        .collect();

    Ok(LogicalPlan::CreateTable { table, columns })
}