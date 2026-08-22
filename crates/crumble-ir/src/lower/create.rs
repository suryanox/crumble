use crate::column::{ColumnDef, ColumnType};
use crate::{LogicalPlan, LowerError};
use sqlparser::ast::{CreateTable, DataType};

pub(super) fn lower_create(create_table: &CreateTable) -> Result<LogicalPlan, LowerError> {
    let table = create_table.name.to_string();

    let columns = create_table
        .columns
        .iter()
        .map(|col| {
            let typ = lower_column_type(&col.data_type)?;
            Ok(ColumnDef {
                name: col.name.value.clone(),
                typ,
            })
        })
        .collect::<Result<Vec<_>, LowerError>>()?;

    Ok(LogicalPlan::CreateTable { table, columns })
}

fn lower_column_type(data_type: &DataType) -> Result<ColumnType, LowerError> {
    match data_type {
        DataType::Int(_) | DataType::Integer(_) | DataType::BigInt(_) => Ok(ColumnType::Int),
        DataType::Bool | DataType::Boolean => Ok(ColumnType::Bool),
        DataType::Text | DataType::Varchar(_) | DataType::String(_) => Ok(ColumnType::Text),
        other => Err(LowerError::Unsupported(format!("column type: {other:?}"))),
    }
}
