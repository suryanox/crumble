use crate::{BinaryOperator, Expr, Literal, LogicalPlan, LowerError};
use crumble_sql::Ast;
use sqlparser::ast::{
    BinaryOperator as SqlBinaryOperator, Expr as SqlExpr, Insert, Select, SelectItem, SetExpr,
    Statement, TableFactor, TableWithJoins, Value as SqlValue,
};

pub fn lower(ast: &Ast) -> Result<LogicalPlan, LowerError> {
    let statement = ast
        .statements
        .first()
        .ok_or_else(|| LowerError::Unsupported("empty statement".to_string()))?;

    lower_statement(statement)
}

fn lower_statement(statement: &Statement) -> Result<LogicalPlan, LowerError> {
    match statement {
        Statement::Query(query) => lower_select_expr(&query.body),
        Statement::Insert(insert) => lower_insert(insert),
        Statement::CreateTable(create_table) => {
            let table = create_table.name.to_string();

            let columns = create_table
                .columns
                .iter()
                .map(|col| col.name.value.clone())
                .collect();

            Ok(LogicalPlan::CreateTable { table, columns })
        }
        other => Err(LowerError::Unsupported(format!("statement: {other:?}"))),
    }
}

fn lower_insert(insert: &Insert) -> Result<LogicalPlan, LowerError> {
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

fn lower_literal_expr(expr: &SqlExpr) -> Result<Literal, LowerError> {
    match expr {
        SqlExpr::Value(value_with_span) => lower_value(&value_with_span.value),
        other => Err(LowerError::Unsupported(format!("VALUES entry: {other:?}"))),
    }
}
fn lower_select_expr(set_expr: &SetExpr) -> Result<LogicalPlan, LowerError> {
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

fn lower_expr(expr: &SqlExpr) -> Result<Expr, LowerError> {
    match expr {
        SqlExpr::Identifier(ident) => Ok(Expr::Column(ident.value.clone())),
        SqlExpr::Value(value_with_span) => lower_value(&value_with_span.value).map(Expr::Literal),
        SqlExpr::BinaryOp { left, op, right } => Ok(Expr::BinaryOp {
            left: Box::new(lower_expr(left)?),
            op: lower_binary_operator(op)?,
            right: Box::new(lower_expr(right)?),
        }),
        other => Err(LowerError::Unsupported(format!("expression: {other:?}"))),
    }
}

fn lower_value(value: &SqlValue) -> Result<Literal, LowerError> {
    match value {
        // hasDecimal seems broken currently, I can't think of a better way currently
        SqlValue::Number(num, _) => {
            if let Ok(value) = num.parse::<i64>() {
                Ok(Literal::Int(value))
            } else if let Ok(value) = num.parse::<f64>() {
                Ok(Literal::Float(value))
            } else {
                Err(LowerError::Unsupported(format!("numeric literal: {num}")))
            }
        }
        SqlValue::Boolean(b) => Ok(Literal::Bool(*b)),
        SqlValue::SingleQuotedString(s) => Ok(Literal::String(s.clone())),
        other => Err(LowerError::Unsupported(format!("literal: {other:?}"))),
    }
}

fn lower_binary_operator(op: &SqlBinaryOperator) -> Result<BinaryOperator, LowerError> {
    match op {
        SqlBinaryOperator::Eq => Ok(BinaryOperator::Eq),
        SqlBinaryOperator::NotEq => Ok(BinaryOperator::NotEq),
        SqlBinaryOperator::Lt => Ok(BinaryOperator::Lt),
        SqlBinaryOperator::LtEq => Ok(BinaryOperator::LtEq),
        SqlBinaryOperator::Gt => Ok(BinaryOperator::Gt),
        SqlBinaryOperator::GtEq => Ok(BinaryOperator::GtEq),
        SqlBinaryOperator::And => Ok(BinaryOperator::And),
        SqlBinaryOperator::Or => Ok(BinaryOperator::Or),
        SqlBinaryOperator::Plus => Ok(BinaryOperator::Add),
        other => Err(LowerError::Unsupported(format!(
            "binary operator: {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::{BinaryOperator, Expr, Literal};
    use crate::plan::LogicalPlan;
    use crumble_sql::parse;

    #[test]
    fn lowers_select_with_filter() -> Result<(), Box<dyn std::error::Error>> {
        let ast = parse("SELECT name FROM users WHERE age > 30")?;
        let plan = lower(&ast)?;

        let expected = LogicalPlan::Project {
            input: Box::new(LogicalPlan::Filter {
                input: Box::new(LogicalPlan::Scan {
                    table: "users".to_string(),
                }),
                predicate: Expr::BinaryOp {
                    left: Box::new(Expr::Column("age".to_string())),
                    op: BinaryOperator::Gt,
                    right: Box::new(Expr::Literal(Literal::Int(30))),
                },
            }),
            columns: vec!["name".to_string()],
        };

        assert_eq!(plan, expected);
        Ok(())
    }

    #[test]
    fn lowers_select_without_filter() -> Result<(), Box<dyn std::error::Error>> {
        let ast = parse("SELECT name FROM users")?;
        let plan = lower(&ast)?;

        let expected = LogicalPlan::Project {
            input: Box::new(LogicalPlan::Scan {
                table: "users".to_string(),
            }),
            columns: vec!["name".to_string()],
        };

        assert_eq!(plan, expected);
        Ok(())
    }

    #[test]
    fn lowers_select_with_filter_float() -> Result<(), Box<dyn std::error::Error>> {
        let ast = parse("SELECT name FROM users WHERE salary > 30.12894")?;
        let plan = lower(&ast)?;

        let expected = LogicalPlan::Project {
            input: Box::new(LogicalPlan::Filter {
                input: Box::new(LogicalPlan::Scan {
                    table: "users".to_string(),
                }),
                predicate: Expr::BinaryOp {
                    left: Box::new(Expr::Column("salary".to_string())),
                    op: BinaryOperator::Gt,
                    right: Box::new(Expr::Literal(Literal::Float(30.12894))),
                },
            }),
            columns: vec!["name".to_string()],
        };

        assert_eq!(plan, expected);
        Ok(())
    }

    #[test]
    fn lowers_float_literal() -> Result<(), Box<dyn std::error::Error>> {
        let ast = parse("SELECT name FROM users WHERE age > 22.5")?;
        let plan = lower(&ast)?;

        match plan {
            LogicalPlan::Project { input, .. } => match *input {
                LogicalPlan::Filter { predicate, .. } => match predicate {
                    Expr::BinaryOp { right, .. } => match *right {
                        Expr::Literal(Literal::Float(f)) => assert_eq!(f, 22.5),
                        other => panic!("expected Float literal, got {other:?}"),
                    },
                    other => panic!("expected Filter, got {other:?}"),
                },
                other => panic!("expected Filter, got {other:?}"),
            },
            other => panic!("expected Project, got {other:?}"),
        }
        Ok(())
    }
}
