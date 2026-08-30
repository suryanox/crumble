use crate::lower::expr::lower_expr;
use crate::plan::{JoinKind, Projection};
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

    let (left_real, left_qualifier) = table_name_and_qualifier(&table_with_joins.relation)?;
    let left_plan = LogicalPlan::Scan {
        table: left_real.clone(),
    };

    match table_with_joins.joins.as_slice() {
        [] => Ok(left_plan),
        [join] => {
            let (right_real, right_qualifier) = table_name_and_qualifier(&join.relation)?;
            let right_plan = LogicalPlan::Scan {
                table: right_real.clone(),
            };

            let (on, kind) = match &join.join_operator {
                sqlparser::ast::JoinOperator::Inner(sqlparser::ast::JoinConstraint::On(expr))
                | sqlparser::ast::JoinOperator::Join(sqlparser::ast::JoinConstraint::On(expr)) => {
                    (lower_expr(expr)?, JoinKind::Inner)
                }
                sqlparser::ast::JoinOperator::Left(sqlparser::ast::JoinConstraint::On(expr)) => {
                    (lower_expr(expr)?, JoinKind::Left)
                }
                sqlparser::ast::JoinOperator::Right(sqlparser::ast::JoinConstraint::On(expr)) => {
                    (lower_expr(expr)?, JoinKind::Right)
                }
                sqlparser::ast::JoinOperator::FullOuter(sqlparser::ast::JoinConstraint::On(
                    expr,
                )) => (lower_expr(expr)?, JoinKind::FullOuter),
                other => return Err(LowerError::Unsupported(format!("join type: {other:?}"))),
            };

            Ok(LogicalPlan::Join {
                left: Box::new(left_plan),
                right: Box::new(right_plan),
                left_table: left_qualifier,
                right_table: right_qualifier,
                on,
                kind,
            })
        }
        _ => Err(LowerError::Unsupported(
            "only a single JOIN is supported for now".to_string(),
        )),
    }
}

fn table_name_and_qualifier(relation: &TableFactor) -> Result<(String, String), LowerError> {
    match relation {
        TableFactor::Table { name, alias, .. } => {
            let real_name = name.to_string();
            let qualifier = alias
                .as_ref()
                .map(|a| a.name.value.clone())
                .unwrap_or_else(|| real_name.clone());
            Ok((real_name, qualifier))
        }
        other => Err(LowerError::Unsupported(format!("FROM entry: {other:?}"))),
    }
}

fn lower_projection(projection: &[SelectItem]) -> Result<Projection, LowerError> {
    if projection.len() == 1 && matches!(projection[0], SelectItem::Wildcard(_)) {
        return Ok(Projection::All);
    }

    let columns = projection
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
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Projection::Columns(columns))
}
