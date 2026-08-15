use crate::{LogicalPlan, LowerError};
use crate::lower::expr::lower_expr;

pub(super) fn lower_delete(delete: &sqlparser::ast::Delete) -> Result<LogicalPlan, LowerError> {
    let table = delete
        .from
        .to_string()
        .trim_start_matches("FROM ")
        .split(',')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();

    let predicate = delete.selection.as_ref().map(lower_expr).transpose()?;

    Ok(LogicalPlan::Delete { table, predicate })
}
