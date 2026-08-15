use crate::lower::expr::{lower_expr, lower_literal_expr};
use crate::{LogicalPlan, LowerError};
use sqlparser::ast::{TableFactor, Update};

pub(super) fn lower_update(update: &Update) -> Result<LogicalPlan, LowerError> {
    let table_name = match &update.table.relation {
        TableFactor::Table { name, .. } => name.to_string(),
        other => return Err(LowerError::Unsupported(format!("UPDATE target: {other:?}"))),
    };

    let assignments = update
        .assignments
        .iter()
        .map(|assignment| {
            let col = assignment.target.to_string();
            let val = lower_literal_expr(&assignment.value)?;
            Ok((col, val))
        })
        .collect::<Result<Vec<_>, LowerError>>()?;

    let predicate = update.selection.as_ref().map(lower_expr).transpose()?;

    Ok(LogicalPlan::Update {
        table: table_name,
        assignments,
        predicate,
    })
}
