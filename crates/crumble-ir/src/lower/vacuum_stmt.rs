use sqlparser::ast::VacuumStatement;

use crate::{LogicalPlan, LowerError};

pub(super) fn lower_vacuum(vacuum: &VacuumStatement) -> Result<LogicalPlan, LowerError> {
    match vacuum.table_name.as_ref() {
        Some(name) => Ok(LogicalPlan::VacuumTable {
            table: name.to_string(),
        }),
        None => Ok(LogicalPlan::VacuumAll),
    }
}
