use sqlparser::ast::{
    Expr as SqlExpr, Function, FunctionArg, FunctionArgExpr, FunctionArguments, GroupByExpr,
    Select, SelectItem,
};

use crate::plan::{AggFunc, AggregateExpr};
use crate::{LogicalPlan, LowerError};

pub(super) fn is_aggregate_query(select: &Select) -> bool {
    if !matches!(select.group_by, GroupByExpr::Expressions(ref exprs, _) if !exprs.is_empty()) {
        // fall through to checking projection below even with no GROUP BY —
        // a bare "SELECT COUNT(*) FROM t" is still an aggregate query.
    }
    select.projection.iter().any(|item| {
        matches!(
            item,
            SelectItem::UnnamedExpr(SqlExpr::Function(_))
                | SelectItem::ExprWithAlias {
                    expr: SqlExpr::Function(_),
                    ..
                }
        )
    }) || matches!(&select.group_by, GroupByExpr::Expressions(exprs, _) if !exprs.is_empty())
}

pub(super) fn lower_aggregate(
    select: &Select,
    input: LogicalPlan,
) -> Result<LogicalPlan, LowerError> {
    let group_by = match &select.group_by {
        GroupByExpr::Expressions(exprs, _) => exprs
            .iter()
            .map(|e| match e {
                SqlExpr::Identifier(ident) => Ok(ident.value.clone()),
                other => Err(LowerError::Unsupported(format!(
                    "GROUP BY expression: {other:?}"
                ))),
            })
            .collect::<Result<Vec<_>, _>>()?,
        GroupByExpr::All(_) => {
            return Err(LowerError::Unsupported(
                "GROUP BY ALL not supported".to_string(),
            ));
        }
    };

    let mut aggregates = Vec::new();
    for item in &select.projection {
        let (expr, explicit_alias) = match item {
            SelectItem::UnnamedExpr(e) => (e, None),
            SelectItem::ExprWithAlias { expr, alias } => (expr, Some(alias.value.clone())),
            other => {
                return Err(LowerError::Unsupported(format!(
                    "aggregate projection item: {other:?}"
                )));
            }
        };

        if let SqlExpr::Function(func) = expr {
            aggregates.push(lower_aggregate_func(func, explicit_alias)?);
        }
        // plain identifiers here are assumed to be GROUP BY columns —
        // no validation that they're actually IN group_by (named gap, see tradeoffs).
    }

    Ok(LogicalPlan::Aggregate {
        input: Box::new(input),
        group_by,
        aggregates,
    })
}

fn lower_aggregate_func(
    func: &Function,
    explicit_alias: Option<String>,
) -> Result<AggregateExpr, LowerError> {
    let name = func.name.to_string().to_uppercase();
    let agg_func = match name.as_str() {
        "COUNT" => AggFunc::Count,
        "SUM" => AggFunc::Sum,
        "AVG" => AggFunc::Avg,
        "MIN" => AggFunc::Min,
        "MAX" => AggFunc::Max,
        other => {
            return Err(LowerError::Unsupported(format!(
                "aggregate function: {other}"
            )));
        }
    };

    let args = match &func.args {
        FunctionArguments::List(list) => &list.args,
        FunctionArguments::None => &Vec::new(),
        FunctionArguments::Subquery(_) => {
            return Err(LowerError::Unsupported(
                "subquery aggregate args not supported".to_string(),
            ));
        }
    };

    let column = match args.as_slice() {
        [] => None,
        [FunctionArg::Unnamed(FunctionArgExpr::Wildcard)] => None, // COUNT(*)
        [FunctionArg::Unnamed(FunctionArgExpr::Expr(SqlExpr::Identifier(ident)))] => {
            Some(ident.value.clone())
        }
        other => {
            return Err(LowerError::Unsupported(format!(
                "aggregate function arg: {other:?}"
            )));
        }
    };

    if column.is_none() && agg_func != AggFunc::Count {
        return Err(LowerError::Unsupported(format!(
            "{name}(*) not supported, {name} needs a column"
        )));
    }

    let alias = explicit_alias.unwrap_or_else(|| match &column {
        Some(col) => format!("{name}({col})"),
        None => format!("{name}(*)"),
    });

    Ok(AggregateExpr {
        func: agg_func,
        column,
        alias,
    })
}
