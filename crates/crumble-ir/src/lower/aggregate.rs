use sqlparser::ast::{
    Expr as SqlExpr, Function, FunctionArg, FunctionArgExpr, FunctionArguments, GroupByExpr,
    Select, SelectItem,
};

use crate::plan::{AggFunc, AggregateExpr};
use crate::{LogicalPlan, LowerError, Projection};

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

    let plan = LogicalPlan::Aggregate {
        input: Box::new(input),
        group_by,
        aggregates,
    };
    Ok((plan, Projection::Columns(output_columns)))
}
