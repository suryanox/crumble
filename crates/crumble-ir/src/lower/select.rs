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
    let base = lower_from(&select.from)?;

    let filtered = match &select.selection {
        Some(predicate) => LogicalPlan::Filter {
            input: Box::new(base),
            predicate: lower_expr(predicate)?,
        },
        None => base,
    };

    let columns = lower_projection(&select.projection)?;

    Ok(LogicalPlan::Project {
        input: Box::new(filtered),
        columns,
    })
}

fn lower_from(from: &[TableWithJoins]) -> Result<LogicalPlan, LowerError> {
    let [table_with_joins] = from else {
        return Err(LowerError::Unsupported(
            "queries must reference exactly one FROM entry".to_string(),
        ));
    };

    let left_table = table_name(&table_with_joins.relation)?;
    let left_plan = LogicalPlan::Scan {
        table: left_table.clone(),
    };

    match table_with_joins.joins.as_slice() {
        [] => Ok(left_plan),
        [join] => {
            let right_table = table_name(&join.relation)?;
            let right_plan = LogicalPlan::Scan {
                table: right_table.clone(),
            };

            let on = match &join.join_operator {
                sqlparser::ast::JoinOperator::Inner(sqlparser::ast::JoinConstraint::On(expr)) => {
                    lower_expr(expr)?
                }
                other => return Err(LowerError::Unsupported(format!("join type: {other:?}"))),
            };

            Ok(LogicalPlan::Join {
                left: Box::new(left_plan),
                right: Box::new(right_plan),
                left_table,
                right_table,
                on,
            })
        }
        _ => Err(LowerError::Unsupported(
            "only a single JOIN is supported for now".to_string(),
        )),
    }
}

fn table_name(relation: &TableFactor) -> Result<String, LowerError> {
    match relation {
        TableFactor::Table { name, .. } => Ok(name.to_string()),
        other => Err(LowerError::Unsupported(format!("FROM entry: {other:?}"))),
    }
}

fn lower_projection(projection: &[SelectItem]) -> Result<Vec<String>, LowerError> {
    projection
        .iter()
        .map(|item| match item {
            SelectItem::UnnamedExpr(SqlExpr::Identifier(ident)) => Ok(ident.value.clone()),
            SelectItem::UnnamedExpr(SqlExpr::CompoundIdentifier(parts)) => Ok(parts
                .iter()
                .map(|p| p.value.as_str())
                .collect::<Vec<_>>()
                .join(".")),
            other => Err(LowerError::Unsupported(format!(
                "projection item: {other:?}"
            ))),
        })
        .collect()
}
