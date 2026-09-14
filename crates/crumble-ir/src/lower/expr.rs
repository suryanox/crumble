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
        SqlExpr::IsNull(inner) => Ok(Expr::IsNull {
            expr: Box::new(lower_expr(inner)?),
            negated: false,
        }),
        SqlExpr::IsNotNull(inner) => Ok(Expr::IsNull {
            expr: Box::new(lower_expr(inner)?),
            negated: true,
        }),
        SqlExpr::CompoundIdentifier(parts) => {
            let name = parts
                .iter()
                .map(|p| p.value.as_str())
                .collect::<Vec<_>>()
                .join(".");
            Ok(Expr::Column(name))
        }
        SqlExpr::Like {
            negated,
            expr,
            pattern,
            ..
        } => Ok(Expr::Like {
            expr: Box::new(lower_expr(expr)?),
            pattern: Box::new(lower_expr(pattern)?),
            negated: *negated,
        }),
        SqlExpr::UnaryOp {
            op: sqlparser::ast::UnaryOperator::Not,
            expr,
        } => Ok(Expr::BinaryOp {
            left: Box::new(lower_expr(expr)?),
            op: BinaryOperator::Eq,
            right: Box::new(Expr::Literal(Literal::Bool(false))),
        }),
        SqlExpr::InList {
            expr,
            list,
            negated,
        } => {
            let target = lower_expr(expr)?;
            let mut items = list.iter().map(lower_expr);

            let first = items
                .next()
                .ok_or_else(|| LowerError::Unsupported("IN () with empty list".to_string()))?;
            let mut acc = Expr::BinaryOp {
                left: Box::new(target.clone()),
                op: BinaryOperator::Eq,
                right: Box::new(first?),
            };

            for item in items {
                acc = Expr::BinaryOp {
                    left: Box::new(acc),
                    op: BinaryOperator::Or,
                    right: Box::new(Expr::BinaryOp {
                        left: Box::new(target.clone()),
                        op: BinaryOperator::Eq,
                        right: Box::new(item?),
                    }),
                };
            }

            if *negated {
                Ok(Expr::BinaryOp {
                    left: Box::new(acc),
                    op: BinaryOperator::Eq,
                    right: Box::new(Expr::Literal(Literal::Bool(false))),
                })
            } else {
                Ok(acc)
            }
        }
        SqlExpr::Between {
            expr,
            negated,
            low,
            high,
        } => {
            let target = lower_expr(expr)?;
            let range = Expr::BinaryOp {
                left: Box::new(Expr::BinaryOp {
                    left: Box::new(target.clone()),
                    op: BinaryOperator::GtEq,
                    right: Box::new(lower_expr(low)?),
                }),
                op: BinaryOperator::And,
                right: Box::new(Expr::BinaryOp {
                    left: Box::new(target),
                    op: BinaryOperator::LtEq,
                    right: Box::new(lower_expr(high)?),
                }),
            };

            if *negated {
                Ok(Expr::BinaryOp {
                    left: Box::new(range),
                    op: BinaryOperator::Eq,
                    right: Box::new(Expr::Literal(Literal::Bool(false))),
                })
            } else {
                Ok(range)
            }
        }
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
        SqlValue::Null => Ok(Literal::Null),
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
