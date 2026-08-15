mod create;
mod delete;
mod expr;
mod insert;
mod select;
mod update;
use crate::lower::create::lower_create;
use crate::lower::delete::lower_delete;
use crate::lower::insert::lower_insert;
use crate::lower::select::lower_select_expr;
use crate::lower::update::lower_update;
use crate::{LogicalPlan, LowerError};
use crumble_sql::Ast;
use sqlparser::ast::Statement;

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
        Statement::CreateTable(create_table) => lower_create(create_table),
        Statement::Delete(delete) => lower_delete(delete),
        Statement::Update(update) => lower_update(update),
        other => Err(LowerError::Unsupported(format!("statement: {other:?}"))),
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
