use crate::error::ExecError;
use crate::row_set::RowSet;
use crumble_ir::{BinaryOperator, Expr, Literal, PhysicalPlan};
use crumble_storage::{Catalog, Row, Value};

pub fn execute(plan: &PhysicalPlan, catalog: &Catalog) -> Result<RowSet, ExecError> {
    match plan {
        PhysicalPlan::SeqScan { table } => {
            let table = catalog.get(table)?;
            Ok(RowSet::new(table.columns().to_vec(), table.rows().to_vec()))
        }
        PhysicalPlan::Filter { input, predicate } => {
            let input = execute(input, catalog)?;
            let mut kept = Vec::new();

            for row in input.rows() {
                let value = eval_expr(predicate, input.columns(), row)?;
                match value {
                    Value::Bool(true) => kept.push(row.clone()),
                    Value::Bool(false) => {}
                    _ => return Err(ExecError::TypeMismatch),
                }
            }

            Ok(RowSet::new(input.columns().to_vec(), kept))
        }
        PhysicalPlan::Project { input, columns } => {
            let input = execute(input, catalog)?;
            let mut indices = Vec::with_capacity(columns.len());

            for column in columns {
                let index = input
                    .column_index(column)
                    .ok_or_else(|| ExecError::ColumnNotFound(column.clone()))?;
                indices.push(index);
            }

            let projected_rows = input
                .rows()
                .iter()
                .map(|row| {
                    let values = indices.iter().map(|&i| row.values()[i].clone()).collect();
                    Row::new(values)
                })
                .collect();

            Ok(RowSet::new(columns.clone(), projected_rows))
        }
    }
}

fn eval_expr(expr: &Expr, columns: &[String], row: &Row) -> Result<Value, ExecError> {
    match expr {
        Expr::Column(name) => {
            let index = columns
                .iter()
                .position(|col| col == name)
                .ok_or_else(|| ExecError::ColumnNotFound(name.clone()))?;
            Ok(row.values()[index].clone())
        }
        Expr::Literal(literal) => Ok(literal_to_value(literal)),
        Expr::BinaryOp { left, op, right } => {
            let left = eval_expr(left, columns, row)?;
            let right = eval_expr(right, columns, row)?;
            eval_binary_op(left, *op, right)
        }
    }
}

fn literal_to_value(literal: &Literal) -> Value {
    match literal {
        Literal::Int(n) => Value::Int(*n),
        Literal::Bool(b) => Value::Bool(*b),
        Literal::String(s) => Value::String(s.clone()),
    }
}

fn eval_binary_op(left: Value, op: BinaryOperator, right: Value) -> Result<Value, ExecError> {
    match (left, right) {
        (Value::Int(l), Value::Int(r)) => eval_int(l, op, r),
        (Value::Bool(l), Value::Bool(r)) => eval_bool(l, op, r),
        (Value::String(l), Value::String(r)) => eval_string(&l, op, &r),
        _ => Err(ExecError::TypeMismatch),
    }
}

fn eval_int(l: i64, op: BinaryOperator, r: i64) -> Result<Value, ExecError> {
    match op {
        BinaryOperator::Eq => Ok(Value::Bool(l == r)),
        BinaryOperator::NotEq => Ok(Value::Bool(l != r)),
        BinaryOperator::Lt => Ok(Value::Bool(l < r)),
        BinaryOperator::LtEq => Ok(Value::Bool(l <= r)),
        BinaryOperator::Gt => Ok(Value::Bool(l > r)),
        BinaryOperator::GtEq => Ok(Value::Bool(l >= r)),
        BinaryOperator::And => Err(ExecError::TypeMismatch),
        BinaryOperator::Or => Err(ExecError::TypeMismatch),
    }
}

fn eval_bool(l: bool, op: BinaryOperator, r: bool) -> Result<Value, ExecError> {
    match op {
        BinaryOperator::Eq => Ok(Value::Bool(l == r)),
        BinaryOperator::NotEq => Ok(Value::Bool(l != r)),
        BinaryOperator::Lt => Err(ExecError::TypeMismatch),
        BinaryOperator::LtEq => Err(ExecError::TypeMismatch),
        BinaryOperator::Gt => Err(ExecError::TypeMismatch),
        BinaryOperator::GtEq => Err(ExecError::TypeMismatch),
        BinaryOperator::And => Ok(Value::Bool(l && r)),
        BinaryOperator::Or => Ok(Value::Bool(l || r)),
    }
}

fn eval_string(l: &str, op: BinaryOperator, r: &str) -> Result<Value, ExecError> {
    match op {
        BinaryOperator::Eq => Ok(Value::Bool(l == r)),
        BinaryOperator::NotEq => Ok(Value::Bool(l != r)),
        BinaryOperator::Lt => Ok(Value::Bool(l < r)),
        BinaryOperator::LtEq => Ok(Value::Bool(l <= r)),
        BinaryOperator::Gt => Ok(Value::Bool(l > r)),
        BinaryOperator::GtEq => Ok(Value::Bool(l >= r)),
        BinaryOperator::And => Err(ExecError::TypeMismatch),
        BinaryOperator::Or => Err(ExecError::TypeMismatch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crumble_ir::{lower, to_physical};
    use crumble_sql::parse;
    use crumble_storage::{Catalog, Row, Value};

    fn seeded_catalog() -> Catalog {
        let mut catalog = Catalog::new();
        catalog
            .create_table("users", vec!["name".to_string(), "age".to_string()])
            .unwrap();

        let users = catalog.get_mut("users").unwrap();
        users
            .insert(Row::new(vec![
                Value::String("alice".to_string()),
                Value::Int(35),
            ]))
            .unwrap();
        users
            .insert(Row::new(vec![
                Value::String("bob".to_string()),
                Value::Int(22),
            ]))
            .unwrap();

        catalog
    }

    #[test]
    fn executes_filtered_projection() -> Result<(), Box<dyn std::error::Error>> {
        let catalog = seeded_catalog();

        let ast = parse("SELECT name FROM users WHERE age > 30")?;
        let logical = lower(&ast)?;
        let physical = to_physical(logical);

        let result = execute(&physical, &catalog)?;

        assert_eq!(result.columns(), &["name".to_string()]);
        assert_eq!(
            result.rows(),
            &[Row::new(vec![Value::String("alice".to_string())])]
        );
        Ok(())
    }

    #[test]
    fn errors_on_unknown_column() {
        let catalog = seeded_catalog();

        let ast = parse("SELECT ghost FROM users").unwrap();
        let logical = lower(&ast).unwrap();
        let physical = to_physical(logical);

        let result = execute(&physical, &catalog);

        assert!(matches!(result, Err(ExecError::ColumnNotFound(col)) if col == "ghost"));
    }
}
