use sqlparser::ast::{
    Expr as SqlExpr, LimitClause, OrderBy, OrderByExpr, OrderByKind, Value as SqlValue,
};

use crate::plan::SortDirection;
use crate::{LogicalPlan, LowerError};

pub(super) fn apply_order_by(
    input: LogicalPlan,
    order_by: &Option<OrderBy>,
) -> Result<LogicalPlan, LowerError> {
    let Some(order_by) = order_by else {
        return Ok(input);
    };

    let exprs: &[OrderByExpr] = match &order_by.kind {
        OrderByKind::Expressions(exprs) => exprs,
        OrderByKind::All(_) => {
            return Err(LowerError::Unsupported(
                "ORDER BY ALL not supported".to_string(),
            ));
        }
    };

    let columns = exprs
        .iter()
        .map(lower_order_by_expr)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(LogicalPlan::Sort {
        input: Box::new(input),
        order_by: columns,
    })
}

fn lower_order_by_expr(expr: &OrderByExpr) -> Result<(String, SortDirection), LowerError> {
    let column = match &expr.expr {
        SqlExpr::Identifier(ident) => ident.value.clone(),
        SqlExpr::CompoundIdentifier(parts) => parts
            .iter()
            .map(|p| p.value.as_str())
            .collect::<Vec<_>>()
            .join("."),
        other => {
            return Err(LowerError::Unsupported(format!(
                "ORDER BY expression: {other:?}"
            )));
        }
    };

    let direction = match expr.options.asc {
        Some(false) => SortDirection::Desc,
        _ => SortDirection::Asc,
    };

    Ok((column, direction))
}

pub(super) fn apply_limit(
    input: LogicalPlan,
    limit_clause: &Option<LimitClause>,
) -> Result<LogicalPlan, LowerError> {
    let Some(clause) = limit_clause else {
        return Ok(input);
    };

    let (limit_expr, offset) = match clause {
        LimitClause::LimitOffset { limit, offset, .. } => (limit.as_ref(), offset.as_ref()),
        other => return Err(LowerError::Unsupported(format!("LIMIT clause: {other:?}"))),
    };

    let limit_value = limit_expr.map(number_value).transpose()?;
    let offset_value = offset.map(|o| number_value(&o.value)).transpose()?;

    Ok(LogicalPlan::Limit {
        input: Box::new(input),
        limit: limit_value,
        offset: offset_value,
    })
}

fn number_value(expr: &SqlExpr) -> Result<u64, LowerError> {
    match expr {
        SqlExpr::Value(v) => match &v.value {
            SqlValue::Number(n, _) => n
                .parse::<u64>()
                .map_err(|_| LowerError::Unsupported(format!("LIMIT/OFFSET value: {n}"))),
            other => Err(LowerError::Unsupported(format!(
                "LIMIT/OFFSET value: {other:?}"
            ))),
        },
        other => Err(LowerError::Unsupported(format!(
            "LIMIT/OFFSET expression: {other:?}"
        ))),
    }
}
