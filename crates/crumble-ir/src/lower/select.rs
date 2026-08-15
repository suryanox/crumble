use crate::lower::expr::lower_expr;
use crate::{LogicalPlan, LowerError};
use sqlparser::ast::{Expr as SqlExpr, Select, SelectItem, SetExpr, TableFactor, TableWithJoins};
pub(super) fn lower_select_expr(set_expr: &SetExpr) -> Result<LogicalPlan, LowerError> {
    match set_expr {
        SetExpr::Select(select) => lower_select(select),
        other => Err(LowerError::Unsupported(format!("query body: {other:?}"))),
    }
}

fn lower_select(select: &Select) -> Result<LogicalPlan, LowerError> {
    let table = lower_from(&select.from)?;
    let scan = LogicalPlan::Scan { table };

    let filtered = match &select.selection {
        Some(predicate) => LogicalPlan::Filter {
            input: Box::new(scan),
            predicate: lower_expr(predicate)?,
        },
        None => scan,
    };

    let columns = lower_projection(&select.projection)?;

    Ok(LogicalPlan::Project {
        input: Box::new(filtered),
        columns,
    })
}

fn lower_from(from: &[TableWithJoins]) -> Result<String, LowerError> {
    let [table_with_joins] = from else {
        return Err(LowerError::Unsupported(
            "queries must reference exactly one table".to_string(),
        ));
    };

    match &table_with_joins.relation {
        TableFactor::Table { name, .. } => Ok(name.to_string()),
        other => Err(LowerError::Unsupported(format!("from clause: {other:?}"))),
    }
}

fn lower_projection(projection: &[SelectItem]) -> Result<Vec<String>, LowerError> {
    projection
        .iter()
        .map(|item| match item {
            SelectItem::UnnamedExpr(SqlExpr::Identifier(ident)) => Ok(ident.value.clone()),
            other => Err(LowerError::Unsupported(format!(
                "projection item: {other:?}"
            ))),
        })
        .collect()
}
