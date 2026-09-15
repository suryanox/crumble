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
        Expr::IsNull { expr, negated } => {
            let value = eval_expr(expr, columns, row)?;
            let is_null = matches!(value, Value::Null);
            Ok(Value::Bool(is_null != *negated))
        }
        Expr::Like {
            expr,
            pattern,
            negated,
        } => {
            let value = eval_expr(expr, columns, row)?;
            let pattern_value = eval_expr(pattern, columns, row)?;

            match (&value, &pattern_value) {
                (Value::Null, _) | (_, Value::Null) => Ok(Value::Null), // three-valued, same as any other comparison
                (Value::String(s), Value::String(p)) => {
                    let matches = like_match(s, p);
                    Ok(Value::Bool(matches != *negated))
                }
                _ => Err(ExecError::TypeMismatch),
            }
        }
    }
}

pub(in crate::interpret) fn literal_to_value(literal: &Literal) -> Value {
    match literal {
        Literal::Int(n) => Value::Int(*n),
        Literal::Bool(b) => Value::Bool(*b),
        Literal::String(s) => Value::String(s.clone()),
        Literal::Float(f) => Value::Float(*f),
        Literal::Null => Value::Null,
    }
}

pub(in crate::interpret) fn eval_binary_op(
    left: Value,
    op: BinaryOperator,
    right: Value,
) -> Result<Value, ExecError> {
    match (&left, &right) {
        (Value::Null, _) | (_, Value::Null) => {
            return eval_binary_op_with_null(left, op, right);
        }
        _ => {}
    }

    match (left, right) {
        (Value::Int(l), Value::Int(r)) => eval_int(l, op, r),
        (Value::Bool(l), Value::Bool(r)) => eval_bool(l, op, r),
        (Value::String(l), Value::String(r)) => eval_string(&l, op, &r),
        (Value::Float(l), Value::Float(r)) => eval_float(l, op, r),
        _ => Err(ExecError::TypeMismatch),
    }
}

fn eval_binary_op_with_null(
    left: Value,
    op: BinaryOperator,
    right: Value,
) -> Result<Value, ExecError> {
    match op {
        // three-valued AND: false short-circuits regardless of the null side
        BinaryOperator::And => match (&left, &right) {
            (Value::Bool(false), _) | (_, Value::Bool(false)) => Ok(Value::Bool(false)),
            _ => Ok(Value::Null),
        },
        // three-valued OR: true short-circuits regardless of the null side
        BinaryOperator::Or => match (&left, &right) {
            (Value::Bool(true), _) | (_, Value::Bool(true)) => Ok(Value::Bool(true)),
            _ => Ok(Value::Null),
        },
        // every other comparison against a null operand is unknown, full stop
        BinaryOperator::Eq
        | BinaryOperator::NotEq
        | BinaryOperator::Lt
        | BinaryOperator::LtEq
        | BinaryOperator::Gt
        | BinaryOperator::GtEq
        | BinaryOperator::Add => Ok(Value::Null),
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

fn like_match(text: &str, pattern: &str) -> bool {
    let text: Vec<char> = text.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    like_match_from(&text, &pattern, 0, 0)
}

fn like_match_from(text: &[char], pattern: &[char], ti: usize, pi: usize) -> bool {
    if pi == pattern.len() {
        return ti == text.len();
    }

    match pattern[pi] {
        '%' => {
            // try matching zero characters here, or consume one char of text
            // and stay on this same '%' — classic backtracking wildcard match.
            like_match_from(text, pattern, ti, pi + 1)
                || (ti < text.len() && like_match_from(text, pattern, ti + 1, pi))
        }
        '_' => ti < text.len() && like_match_from(text, pattern, ti + 1, pi + 1),
        c => ti < text.len() && text[ti] == c && like_match_from(text, pattern, ti + 1, pi + 1),
    }
}

#[cfg(test)]
mod like_tests {
    use super::like_match;

    #[test]
    fn percent_matches_anything() {
        assert!(like_match("hello", "%"));
        assert!(like_match("", "%"));
        assert!(like_match("hello world", "hello%"));
        assert!(like_match("hello world", "%world"));
        assert!(like_match("hello world", "%lo wo%"));
        assert!(!like_match("hello", "hi%"));
    }

    #[test]
    fn underscore_matches_exactly_one_char() {
        assert!(like_match("cat", "c_t"));
        assert!(!like_match("ct", "c_t"));
        assert!(!like_match("caat", "c_t"));
    }

    #[test]
    fn combined_wildcards() {
        assert!(like_match("cataract", "c_t%"));
        assert!(!like_match("dog", "c_t%"));
    }

    #[test]
    fn no_wildcards_requires_exact_match() {
        assert!(like_match("exact", "exact"));
        assert!(!like_match("exact", "exactly"));
    }
}
