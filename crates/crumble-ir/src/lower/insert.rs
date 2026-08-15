use sqlparser::ast::{Insert, SetExpr};
use crate::{Literal, LogicalPlan, LowerError};
use crate::lower::expr::lower_literal_expr;

pub(super) fn lower_insert(insert: &Insert) -> Result<LogicalPlan, LowerError> {
    let table = insert.table.to_string();

    let columns = insert
        .columns
        .iter()
        .map(|ident| {
            ident
                .0
                .last()
                .map(|v| {
                    v.as_ident()
                        .ok_or_else(|| LowerError::Unsupported("invalid column".to_string()))
                        .map(|p| p.value.to_string())
                })
                .ok_or_else(|| LowerError::Unsupported("empty column".to_string()))?
        })
        .collect::<Result<Vec<_>, _>>()?;

    let query = insert
        .source
        .as_ref()
        .ok_or_else(|| LowerError::Unsupported("INSERT without VALUES".to_string()))?;

    let values = match query.body.as_ref() {
        SetExpr::Values(values) => values,
        other => return Err(LowerError::Unsupported(format!("INSERT source: {other:?}"))),
    };

    let rows = values
        .rows
        .iter()
        .map(|row| row.iter().map(lower_literal_expr).collect())
        .collect::<Result<Vec<Vec<Literal>>, LowerError>>()?;

    Ok(LogicalPlan::Insert {
        table,
        columns,
        rows,
    })
}