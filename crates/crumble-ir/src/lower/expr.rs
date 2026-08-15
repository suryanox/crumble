use crate::{BinaryOperator, Expr, Literal, LowerError};
use sqlparser::ast::{BinaryOperator as SqlBinaryOperator, Expr as SqlExpr, Value as SqlValue};
pub(in crate::lower) fn lower_literal_expr(expr: &SqlExpr) -> Result<Literal, LowerError> {
    match expr {
        SqlExpr::Value(value_with_span) => lower_value(&value_with_span.value),
        other => Err(LowerError::Unsupported(format!("VALUES entry: {other:?}"))),
    }
}

pub(in crate::lower) fn lower_expr(expr: &SqlExpr) -> Result<Expr, LowerError> {
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

pub(in crate::lower) fn lower_value(value: &SqlValue) -> Result<Literal, LowerError> {
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

pub(in crate::lower) fn lower_binary_operator(
    op: &SqlBinaryOperator,
) -> Result<BinaryOperator, LowerError> {
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
