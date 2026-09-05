use sqlparser::ast::{
    Expr as SqlExpr, Function, FunctionArg, FunctionArgExpr, FunctionArguments, GroupByExpr,
    Select, SelectItem,
};

use crate::lower::expr::{lower_binary_operator, lower_value};
use crate::plan::{AggFunc, AggregateExpr};
use crate::{Expr, LogicalPlan, LowerError, Projection};

pub(super) fn is_aggregate_query(select: &Select) -> bool {
    let has_group_by =
        matches!(&select.group_by, GroupByExpr::Expressions(exprs, _) if !exprs.is_empty());
    let has_aggregate_function = select.projection.iter().any(|item| {
        matches!(
            item,
            SelectItem::UnnamedExpr(SqlExpr::Function(_))
                | SelectItem::ExprWithAlias {
                    expr: SqlExpr::Function(_),
                    ..
                }
        )
    });

    has_group_by || has_aggregate_function
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

pub(super) fn lower_aggregate(
    select: &Select,
    input: LogicalPlan,
) -> Result<(LogicalPlan, Projection), LowerError> {
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
    let mut output_columns = Vec::new();

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

        match expr {
            SqlExpr::Function(func) => {
                let agg = lower_aggregate_func(func, explicit_alias)?;
                output_columns.push(agg.alias.clone());
                aggregates.push(agg);
            }
            SqlExpr::Identifier(ident) => {
                // assumed to be a GROUP BY column — no validation that it's
                // actually IN group_by, named gap, see tradeoffs.md.
                output_columns.push(ident.value.clone());
            }
            other => {
                return Err(LowerError::Unsupported(format!(
                    "SELECT item in aggregate query: {other:?}"
                )));
            }
        }
    }

    let mut having_only_aggregates = Vec::new();
    if let Some(having_expr) = &select.having {
        collect_having_aggregates(having_expr, &aggregates, &mut having_only_aggregates)?;
    }
    let all_aggregates: Vec<AggregateExpr> = aggregates
        .into_iter()
        .chain(having_only_aggregates)
        .collect();

    let aggregate_plan = LogicalPlan::Aggregate {
        input: Box::new(input),
        group_by,
        aggregates: all_aggregates,
    };

    let plan = match &select.having {
        Some(having_expr) => LogicalPlan::Filter {
            input: Box::new(aggregate_plan),
            predicate: lower_having_predicate(having_expr)?,
        },
        None => aggregate_plan,
    };

    Ok((plan, Projection::Columns(output_columns)))
}

/// Walks a HAVING predicate, collecting any aggregate function calls not
/// already present (by alias) among `existing` — these must still be
/// computed by the Aggregate node even though they won't appear in the
/// final SELECT output (e.g. `... HAVING SUM(age) > 100` with no SUM(age)
/// in the SELECT list at all).
fn collect_having_aggregates(
    expr: &SqlExpr,
    existing: &[AggregateExpr],
    collected: &mut Vec<AggregateExpr>,
) -> Result<(), LowerError> {
    match expr {
        SqlExpr::Function(func) => {
            let agg = lower_aggregate_func(func, None)?;
            let already_present = existing
                .iter()
                .chain(collected.iter())
                .any(|a| a.alias == agg.alias);
            if !already_present {
                collected.push(agg);
            }
            Ok(())
        }
        SqlExpr::BinaryOp { left, right, .. } => {
            collect_having_aggregates(left, existing, collected)?;
            collect_having_aggregates(right, existing, collected)
        }
        SqlExpr::IsNull(inner) | SqlExpr::IsNotNull(inner) => {
            collect_having_aggregates(inner, existing, collected)
        }
        _ => Ok(()),
    }
}

/// Lowers a HAVING predicate, converting each aggregate function call into
/// a reference to its already-computed output column (same alias
/// convention as lower_aggregate_func) instead of a fresh function call —
/// by the time HAVING runs, Aggregate has already computed it.
fn lower_having_predicate(expr: &SqlExpr) -> Result<Expr, LowerError> {
    match expr {
        SqlExpr::Function(func) => {
            let agg = lower_aggregate_func(func, None)?;
            Ok(Expr::Column(agg.alias))
        }
        SqlExpr::Identifier(ident) => Ok(Expr::Column(ident.value.clone())),
        SqlExpr::CompoundIdentifier(parts) => Ok(Expr::Column(
            parts
                .iter()
                .map(|p| p.value.as_str())
                .collect::<Vec<_>>()
                .join("."),
        )),
        SqlExpr::Value(value_with_span) => lower_value(&value_with_span.value).map(Expr::Literal),
        SqlExpr::BinaryOp { left, op, right } => Ok(Expr::BinaryOp {
            left: Box::new(lower_having_predicate(left)?),
            op: lower_binary_operator(op)?,
            right: Box::new(lower_having_predicate(right)?),
        }),
        SqlExpr::IsNull(inner) => Ok(Expr::IsNull {
            expr: Box::new(lower_having_predicate(inner)?),
            negated: false,
        }),
        SqlExpr::IsNotNull(inner) => Ok(Expr::IsNull {
            expr: Box::new(lower_having_predicate(inner)?),
            negated: true,
        }),
        other => Err(LowerError::Unsupported(format!(
            "HAVING expression: {other:?}"
        ))),
    }
}
