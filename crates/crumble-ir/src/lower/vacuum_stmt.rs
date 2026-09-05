use sqlparser::ast::VacuumStatement;

use crate::{LogicalPlan, LowerError};

pub(super) fn lower_vacuum(vacuum: &VacuumStatement) -> Result<LogicalPlan, LowerError> {
    let table = vacuum
        .table_name
        .as_ref()
        .ok_or_else(|| LowerError::Unsupported("VACUUM requires a table name".to_string()))?
        .to_string();

    Ok(LogicalPlan::VacuumTable { table })
}
