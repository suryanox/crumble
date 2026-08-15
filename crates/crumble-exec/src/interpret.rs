use crumble_ir::{BinaryOperator, Expr, Literal, PhysicalPlan};
use crumble_storage::{Catalog, Row, Value};

use crate::error::ExecError;
use crate::row_set::RowSet;

pub fn execute(plan: &PhysicalPlan, catalog: &mut Catalog) -> Result<RowSet, ExecError> {
    match plan {
        PhysicalPlan::SeqScan { table } => {
            let table = catalog.get_mut(table)?;
            let rows = table.rows()?;
            Ok(RowSet::new(table.columns().to_vec(), rows))
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
        PhysicalPlan::Insert {
            table,
            columns,
            rows,
        } => {
            let target = catalog.get_mut(table)?;
            let table_columns = target.columns().to_vec();

            for row in rows {
                let values: Vec<Value> = row.iter().map(literal_to_value).collect();
                let ordered = order_row_values(&table_columns, columns, values)?;
                target.insert(Row::new(ordered))?;
            }

            Ok(RowSet::new(Vec::new(), Vec::new()))
        }
        PhysicalPlan::CreateTable { table, columns } => {
            catalog.create_table(table, columns.clone())?;
            Ok(RowSet::new(Vec::new(), Vec::new()))
        }
        PhysicalPlan::Delete { table, predicate } => {
            let target = catalog.get_mut(table)?;
            let located_rows = target.rows_with_location()?;

            let mut deleted = 0;

            for ((page_index, slot), row) in located_rows {
                let matches = match predicate {
                    Some(expr) => {
                        matches!(eval_expr(expr, target.columns(), &row)?, Value::Bool(true))
                    }
                    None => true,
                };

                if matches {
                    target.delete_at(page_index, slot)?;
                    deleted += 1;
                }
            }

            Ok(RowSet::new(
                vec!["deleted".to_string()],
                vec![Row::new(vec![Value::Int(deleted)])],
            ))
        }
        PhysicalPlan::Update {
            table,
            assignments,
            predicate,
        } => {
            let target = catalog.get_mut(table)?;
            let columns = target.columns().to_vec();
            let located_rows = target.rows_with_location()?;

            let mut updated = 0;

            for ((page_index, slot), row) in located_rows {
                let matches = match predicate {
                    Some(expr) => matches!(eval_expr(expr, &columns, &row)?, Value::Bool(true)),
                    None => true,
                };

                if !matches {
                    continue;
                }

                let mut values = row.values().to_vec();

                for (col, literal) in assignments {
                    let index = columns
                        .iter()
                        .position(|col| col == col)
                        .ok_or_else(|| ExecError::ColumnNotFound(col.clone()))?;
                    values[index] = literal_to_value(literal);
                }

                target.delete_at(page_index, slot)?;
                target.insert(Row::new(values))?;
                updated += 1;
            }

            Ok(RowSet::new(
                vec!["updated".to_string()],
                vec![Row::new(vec![Value::Int(updated)])],
            ))
        }
    }
}

fn order_row_values(
    table_columns: &[String],
    insert_columns: &[String],
    values: Vec<Value>,
) -> Result<Vec<Value>, ExecError> {
    if insert_columns.is_empty() {
        return Ok(values);
    }

    if insert_columns.len() != table_columns.len() {
        return Err(ExecError::MissingColumn(
            "INSERT must specify all columns until NULL/defaults are supported".to_string(),
        ));
    }

    table_columns
        .iter()
        .map(|table_col| {
            let index = insert_columns
                .iter()
                .position(|c| c == table_col)
                .ok_or_else(|| ExecError::MissingColumn(table_col.clone()))?;
            Ok(values[index].clone())
        })
        .collect()
}

fn eval_expr(expr: &Expr, columns: &[String], row: &Row) -> Result<Value, ExecError> {
    match expr {
        Expr::Column(name) => {
            let index = columns
                .iter()
                .position(|c| c == name)
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
        Literal::Float(f) => Value::Float(*f),
    }
}

fn eval_binary_op(left: Value, op: BinaryOperator, right: Value) -> Result<Value, ExecError> {
    match (left, right) {
        (Value::Int(l), Value::Int(r)) => eval_int(l, op, r),
        (Value::Bool(l), Value::Bool(r)) => eval_bool(l, op, r),
        (Value::String(l), Value::String(r)) => eval_string(&l, op, &r),
        (Value::Float(l), Value::Float(r)) => eval_float(l, op, r),
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
        BinaryOperator::Add => Ok(Value::Int(l + r)),
        BinaryOperator::And | BinaryOperator::Or => Err(ExecError::TypeMismatch),
    }
}

fn eval_bool(l: bool, op: BinaryOperator, r: bool) -> Result<Value, ExecError> {
    match op {
        BinaryOperator::Eq => Ok(Value::Bool(l == r)),
        BinaryOperator::NotEq => Ok(Value::Bool(l != r)),
        BinaryOperator::And => Ok(Value::Bool(l && r)),
        BinaryOperator::Or => Ok(Value::Bool(l || r)),
        BinaryOperator::Lt
        | BinaryOperator::LtEq
        | BinaryOperator::Gt
        | BinaryOperator::GtEq
        | BinaryOperator::Add => Err(ExecError::TypeMismatch),
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
        BinaryOperator::And | BinaryOperator::Or | BinaryOperator::Add => {
            Err(ExecError::TypeMismatch)
        }
    }
}

fn eval_float(l: f64, op: BinaryOperator, r: f64) -> Result<Value, ExecError> {
    match op {
        BinaryOperator::Eq => Ok(Value::Bool(l == r)),
        BinaryOperator::NotEq => Ok(Value::Bool(l != r)),
        BinaryOperator::Lt => Ok(Value::Bool(l < r)),
        BinaryOperator::LtEq => Ok(Value::Bool(l <= r)),
        BinaryOperator::Gt => Ok(Value::Bool(l > r)),
        BinaryOperator::GtEq => Ok(Value::Bool(l >= r)),
        BinaryOperator::Add => Ok(Value::Float(l + r)),
        BinaryOperator::And | BinaryOperator::Or => Err(ExecError::TypeMismatch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crumble_ir::{lower, to_physical};
    use crumble_sql::parse;
    use crumble_storage::{Catalog, Row, Value};

    fn seeded_catalog() -> (tempfile::TempDir, Catalog) {
        let dir = tempfile::tempdir().unwrap();
        let mut catalog = Catalog::open(dir.path()).unwrap();
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
            .create_table("metrics", vec!["label".to_string(), "score".to_string()])
            .unwrap();

        let metrics = catalog.get_mut("metrics").unwrap();
        metrics
            .insert(Row::new(vec![
                Value::String("a".to_string()),
                Value::Float(4.0),
            ]))
            .unwrap();

        (dir, catalog)
    }

    #[test]
    fn executes_filtered_projection() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, mut catalog) = seeded_catalog();

        let ast = parse("SELECT name FROM users WHERE age > 30")?;
        let logical = lower(&ast)?;
        let physical = to_physical(logical);

        let result = execute(&physical, &mut catalog)?;

        assert_eq!(result.columns(), &["name".to_string()]);
        assert_eq!(
            result.rows(),
            &[Row::new(vec![Value::String("alice".to_string())])]
        );
        Ok(())
    }

    #[test]
    fn errors_on_unknown_column() {
        let (_dir, mut catalog) = seeded_catalog();

        let ast = parse("SELECT ghost FROM users").unwrap();
        let logical = lower(&ast).unwrap();
        let physical = to_physical(logical);

        let result = execute(&physical, &mut catalog);

        assert!(matches!(result, Err(ExecError::ColumnNotFound(col)) if col == "ghost"));
    }

    #[test]
    fn inserts_then_reads_back() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, mut catalog) = seeded_catalog();

        let insert_ast = parse("INSERT INTO users (name, age) VALUES ('eve', 41)")?;
        let insert_physical = to_physical(lower(&insert_ast)?);
        execute(&insert_physical, &mut catalog)?;

        let select_ast = parse("SELECT name FROM users WHERE age > 40")?;
        let select_physical = to_physical(lower(&select_ast)?);
        let result = execute(&select_physical, &mut catalog)?;

        assert_eq!(
            result.rows(),
            &[Row::new(vec![Value::String("eve".to_string())])]
        );
        Ok(())
    }

    #[test]
    fn filters_float_values() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, mut catalog) = seeded_catalog();

        let ast = parse("SELECT label FROM metrics WHERE score > 3.0")?;
        let logical = lower(&ast)?;
        let physical = to_physical(logical);
        let result = execute(&physical, &mut catalog)?;

        assert_eq!(result.rows(), &[Row::new(vec![Value::String("a".into())])]);
        Ok(())
    }

    #[test]
    fn executes_int_addition() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, mut catalog) = seeded_catalog();
        let ast = parse("SELECT name FROM users WHERE age > 20 + 1")?;
        let logical = lower(&ast)?;
        let physical = to_physical(logical);
        let result = execute(&physical, &mut catalog)?;

        assert_eq!(result.rows().len(), 2);
        Ok(())
    }
}
