use crate::ExecError;
use crumble_ir::{BinaryOperator, Expr, Literal};
use crumble_storage::{Row, Value};

pub(in crate::interpret) fn eval_expr(
    expr: &Expr,
    columns: &[String],
    row: &Row,
) -> Result<Value, ExecError> {
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

pub(in crate::interpret) fn literal_to_value(literal: &Literal) -> Value {
    match literal {
        Literal::Int(n) => Value::Int(*n),
        Literal::Bool(b) => Value::Bool(*b),
        Literal::String(s) => Value::String(s.clone()),
        Literal::Float(f) => Value::Float(*f),
    }
}

pub(in crate::interpret) fn eval_binary_op(
    left: Value,
    op: BinaryOperator,
    right: Value,
) -> Result<Value, ExecError> {
    match (left, right) {
        (Value::Int(l), Value::Int(r)) => eval_int(l, op, r),
        (Value::Bool(l), Value::Bool(r)) => eval_bool(l, op, r),
        (Value::String(l), Value::String(r)) => eval_string(&l, op, &r),
        (Value::Float(l), Value::Float(r)) => eval_float(l, op, r),
        _ => Err(ExecError::TypeMismatch),
    }
}

pub(in crate::interpret) fn eval_int(
    l: i64,
    op: BinaryOperator,
    r: i64,
) -> Result<Value, ExecError> {
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

pub(in crate::interpret) fn eval_bool(
    l: bool,
    op: BinaryOperator,
    r: bool,
) -> Result<Value, ExecError> {
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

pub(in crate::interpret) fn eval_string(
    l: &str,
    op: BinaryOperator,
    r: &str,
) -> Result<Value, ExecError> {
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

pub(in crate::interpret) fn eval_float(
    l: f64,
    op: BinaryOperator,
    r: f64,
) -> Result<Value, ExecError> {
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
